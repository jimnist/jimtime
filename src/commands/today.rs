use super::Command;
use anyhow::Result;
use clap::Args;

use crate::store::Day;
use crate::timeutil;
use crate::view::render_day;

/// Print today's time log
#[derive(Args)]
pub struct Today {
    /// Create an empty store for today if none exists
    #[arg(long)]
    create: bool,
}

#[async_trait::async_trait]
impl Command for Today {
    async fn run(&self) -> Result<()> {
        let date = timeutil::today();

        match Day::load(&date)? {
            Some(day) => print!("{}", render_day(&day)),
            None => {
                if self.create {
                    let day = Day {
                        date: date.clone(),
                        sections: Vec::new(),
                    };
                    day.save()?;
                    print!("{}", render_day(&day));
                } else {
                    println!("No log yet for {date}. Add time with `jimtime add`, or pass --create.");
                }
            }
        }
        Ok(())
    }
}
