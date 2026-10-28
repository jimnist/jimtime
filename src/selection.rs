//! A reusable clap argument group for filtering sections by client/project/repo.

use clap::Args;

use crate::repo;
use crate::store::Section;

#[derive(Args, Default)]
pub struct FilterArgs {
    /// Only sections for this client: its key or its name (case-insensitive)
    #[arg(long)]
    client: Option<String>,
    /// Only sections for this project: its key or its name (case-insensitive)
    #[arg(long)]
    project: Option<String>,
    /// Only sections for this repo path
    #[arg(long)]
    repo: Option<String>,
}

impl FilterArgs {
    /// Whether a section passes all provided filters.
    pub fn matches(&self, s: &Section) -> bool {
        if let Some(c) = &self.client
            && !(s.client.eq_ignore_ascii_case(c) || s.client_name.eq_ignore_ascii_case(c))
        {
            return false;
        }
        if let Some(p) = &self.project
            && !(s.project.eq_ignore_ascii_case(p) || s.project_name.eq_ignore_ascii_case(p))
        {
            return false;
        }
        if let Some(r) = &self.repo
            && repo::canonical(r.into()) != repo::canonical(s.repo_path.clone().into())
        {
            return false;
        }
        true
    }
}
