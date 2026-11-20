//! Command line interface.

use clap::{Parser, ValueEnum};

use crate::i18n::Language;

/// Language accepted on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LanguageArg {
    /// Russian
    Ru,
    /// English
    En,
}

/// LAMZU Battery: a battery indicator for LAMZU mice in the Windows tray.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "LamzuBattery",
    version,
    about = "LAMZU Battery - battery indicator for LAMZU mice",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Verbose log
    #[arg(long)]
    pub debug: bool,

    /// Print HID diagnostics and exit
    #[arg(long)]
    pub diagnose: bool,

    /// Also print the raw HID reports
    #[arg(long, requires = "diagnose")]
    pub raw: bool,

    /// Interface language for diagnostics
    #[arg(long, value_enum)]
    pub lang: Option<LanguageArg>,

    /// Internal: wait for the previous instance to release the tray mutex
    #[arg(long, hide = true)]
    pub wait_for_instance: bool,
}

impl Cli {
    /// Explicit language requested on the command line.
    #[must_use]
    pub fn language(&self) -> Option<Language> {
        self.lang.map(|language| match language {
            LanguageArg::Ru => Language::Ru,
            LanguageArg::En => Language::En,
        })
    }
}
