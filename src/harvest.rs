//! Minimal Harvest v2 API client.
//!
//! Credentials come only from the environment [ADR-0003]. Reference data
//! (clients, projects, task assignments) is read to help build the mapping;
//! Phase 3 adds time-entry creation.

use anyhow::{Context, Result, bail};
use reqwest::header::{AUTHORIZATION, USER_AGENT};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const BASE: &str = "https://api.harvestapp.com/v2";
const DEFAULT_USER_AGENT: &str = concat!(
    "jimtime/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/jimnist/jimtime)"
);

pub struct HarvestApi {
    http: reqwest::Client,
    token: String,
    account_id: String,
    user_agent: String,
}

#[derive(Deserialize)]
pub struct ClientRef {
    pub id: u64,
    pub name: String,
}

#[derive(Deserialize)]
pub struct Client {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub is_active: bool,
}

#[derive(Deserialize)]
pub struct Project {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub is_active: bool,
    pub client: ClientRef,
}

#[derive(Deserialize)]
pub struct TaskRef {
    pub id: u64,
    pub name: String,
}

#[derive(Deserialize)]
pub struct TaskAssignment {
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub billable: bool,
    pub task: TaskRef,
}

/// One row of the Harvest uninvoiced report: a project's billable time and
/// expenses that have not been put on an invoice yet.
#[derive(Deserialize)]
pub struct UninvoicedRow {
    pub client_name: String,
    pub currency: String,
    #[serde(default)]
    pub uninvoiced_hours: f64,
    #[serde(default)]
    pub uninvoiced_amount: f64,
    #[serde(default)]
    pub uninvoiced_expenses: f64,
}

#[derive(Deserialize, Default)]
struct Links {
    next: Option<String>,
}

/// A paginated list response. Each endpoint names its array differently, so we
/// implement this per page type.
trait Page<T> {
    fn take_items(self) -> Vec<T>;
    fn next_url(&self) -> Option<String>;
}

macro_rules! page {
    ($name:ident, $field:ident, $item:ty) => {
        #[derive(Deserialize)]
        struct $name {
            $field: Vec<$item>,
            #[serde(default)]
            links: Links,
        }
        impl Page<$item> for $name {
            fn take_items(self) -> Vec<$item> {
                self.$field
            }
            fn next_url(&self) -> Option<String> {
                self.links.next.clone()
            }
        }
    };
}

page!(ClientsPage, clients, Client);
page!(ProjectsPage, projects, Project);
page!(TaskAssignmentsPage, task_assignments, TaskAssignment);
page!(UninvoicedPage, results, UninvoicedRow);

impl HarvestApi {
    /// Build a client from environment credentials, failing loudly if unset.
    pub fn from_env() -> Result<Self> {
        let token = require_env("HARVEST_ACCESS_TOKEN")?;
        let account_id = require_env("HARVEST_ACCOUNT_ID")?;
        let user_agent =
            std::env::var("HARVEST_USER_AGENT").unwrap_or_else(|_| DEFAULT_USER_AGENT.to_string());
        Ok(Self {
            http: reqwest::Client::new(),
            token,
            account_id,
            user_agent,
        })
    }

    /// Attach the auth headers required on every Harvest request.
    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.header(AUTHORIZATION, format!("Bearer {}", self.token))
            .header("Harvest-Account-Id", &self.account_id)
            .header(USER_AGENT, &self.user_agent)
    }

    async fn get<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let resp = self
            .auth(self.http.get(url))
            .send()
            .await
            .with_context(|| format!("requesting {url}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            bail!("Harvest API returned {status} for {url}\n{body}");
        }
        resp.json::<T>()
            .await
            .with_context(|| format!("parsing response from {url}"))
    }

    /// Follow `links.next` until exhausted, collecting all items.
    async fn paged<P, T>(&self, first_url: String) -> Result<Vec<T>>
    where
        P: DeserializeOwned + Page<T>,
    {
        let mut out = Vec::new();
        let mut url = Some(first_url);
        while let Some(u) = url {
            let page: P = self.get(&u).await?;
            url = page.next_url();
            out.extend(page.take_items());
        }
        Ok(out)
    }

    pub async fn list_clients(&self, active_only: bool) -> Result<Vec<Client>> {
        let mut url = format!("{BASE}/clients?per_page=2000");
        if active_only {
            url.push_str("&is_active=true");
        }
        self.paged::<ClientsPage, Client>(url).await
    }

    pub async fn list_projects(&self, active_only: bool) -> Result<Vec<Project>> {
        let mut url = format!("{BASE}/projects?per_page=2000");
        if active_only {
            url.push_str("&is_active=true");
        }
        self.paged::<ProjectsPage, Project>(url).await
    }

    pub async fn task_assignments(&self, project_id: u64) -> Result<Vec<TaskAssignment>> {
        let url = format!("{BASE}/projects/{project_id}/task_assignments?per_page=2000");
        self.paged::<TaskAssignmentsPage, TaskAssignment>(url).await
    }

    /// The uninvoiced report over an inclusive `YYYY-MM-DD` date range: one row
    /// per project with billable time and expenses not yet invoiced. Harvest
    /// rejects a range wider than a year, so callers chunk long spans.
    pub async fn uninvoiced_report(&self, from: &str, to: &str) -> Result<Vec<UninvoicedRow>> {
        let url = format!("{BASE}/reports/uninvoiced?from={from}&to={to}&per_page=2000");
        self.paged::<UninvoicedPage, UninvoicedRow>(url).await
    }

    /// Whether the account tracks time by duration (vs. start/end timestamps).
    /// The `hours` create method only works in duration mode.
    pub async fn tracks_by_duration(&self) -> Result<bool> {
        let company: Company = self.get(&format!("{BASE}/company")).await?;
        Ok(!company.wants_timestamp_timers)
    }

    /// Create a time entry via duration, returning its Harvest id.
    pub async fn create_time_entry(
        &self,
        project_id: u64,
        task_id: u64,
        spent_date: &str,
        hours: f64,
        notes: &str,
    ) -> Result<u64> {
        let url = format!("{BASE}/time_entries");
        let body = NewTimeEntry {
            project_id,
            task_id,
            spent_date,
            hours,
            notes,
        };
        let resp = self
            .auth(self.http.post(&url))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("creating time entry at {url}"))?;

        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().await.unwrap_or_default();
            bail!("Harvest API returned {status} creating a time entry\n{detail}");
        }
        let created: CreatedTimeEntry = resp
            .json()
            .await
            .context("parsing created time entry response")?;
        Ok(created.id)
    }
}

#[derive(Deserialize)]
struct Company {
    #[serde(default)]
    wants_timestamp_timers: bool,
}

#[derive(Serialize)]
struct NewTimeEntry<'a> {
    project_id: u64,
    task_id: u64,
    spent_date: &'a str,
    hours: f64,
    notes: &'a str,
}

#[derive(Deserialize)]
struct CreatedTimeEntry {
    id: u64,
}

fn require_env(name: &str) -> Result<String> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => bail!(
            "{name} is not set.\n\
             Set your Harvest credentials in your shell/dotfiles:\n  \
             export HARVEST_ACCESS_TOKEN=...\n  export HARVEST_ACCOUNT_ID=...\n\
             Create a Personal Access Token at https://id.getharvest.com/developers"
        ),
    }
}
