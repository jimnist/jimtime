use anyhow::Result;

macro_rules! re_export {
    ( $( $md:tt )+ ) => {
        $(
            mod $md;
            pub use $md::*;
        )*
    };
}

// List the names of your command modules to re-export them
// in this module.
re_export! {
    status
    map
    add
    today
    review
    approve
    unapprove
    report
    invoice
    harvest
    cloud
    config
    data
}

pub trait Command {
    async fn run(&self) -> Result<()>;
}

#[macro_export]
macro_rules! register_commands {
    ( $( $command:tt )+ ) => {
        #[derive(clap::Subcommand)]
        enum Commands {
            $(
                $command($command),
            )*
        }

        impl Commands {
            async fn run(&self) -> anyhow::Result<()> {
                use $crate::commands::Command;
                match self {
                    $(
                        Self::$command(c) => c.run().await,
                    )*
                }
            }
        }
    };
}
