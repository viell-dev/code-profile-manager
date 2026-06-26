//! User-facing terminal output and interactive prompts.
//!
//! All direct stdout/stderr printing in the program is funneled through this
//! module so the `print_stdout` / `print_stderr` lints stay scoped to one place
//! and the rest of the code talks in terms of intent (`heading`, `warn`, ...).

use std::fmt::Display;

use anyhow::Result;
use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, Select};

/// Print a blank line followed by an emphasized section heading.
#[expect(clippy::print_stdout, reason = "central user-facing CLI output")]
pub fn heading(text: impl Display) {
    println!("\n{text}");
    println!("{}", "-".repeat(text.to_string().chars().count().min(60)));
}

/// Print a normal informational line.
#[expect(clippy::print_stdout, reason = "central user-facing CLI output")]
pub fn info(text: impl Display) {
    println!("{text}");
}

/// Print an indented detail line under an item.
#[expect(clippy::print_stdout, reason = "central user-facing CLI output")]
pub fn detail(text: impl Display) {
    println!("    {text}");
}

/// Print a bulleted list item.
#[expect(clippy::print_stdout, reason = "central user-facing CLI output")]
pub fn bullet(text: impl Display) {
    println!("  - {text}");
}

/// Print a warning to stderr.
#[expect(clippy::print_stderr, reason = "central user-facing CLI output")]
pub fn warn(text: impl Display) {
    eprintln!("warning: {text}");
}

/// Print an error to stderr.
#[expect(clippy::print_stderr, reason = "central user-facing CLI output")]
pub fn error(text: impl Display) {
    eprintln!("error: {text}");
}

/// Prompt the user to choose one of `items`; returns the selected index.
pub fn select(prompt: &str, items: &[String]) -> Result<usize> {
    let index = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .items(items)
        .default(0)
        .interact()?;
    Ok(index)
}

/// Yes/no confirmation with an explicit default.
pub fn confirm(prompt: &str, default: bool) -> Result<bool> {
    let answer = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .default(default)
        .interact()?;
    Ok(answer)
}

/// Free-text input.
pub fn input(prompt: &str) -> Result<String> {
    let text = Input::with_theme(&ColorfulTheme::default())
        .with_prompt(prompt)
        .interact_text()?;
    Ok(text)
}
