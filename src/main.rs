mod app;
mod ldap;
mod model;
mod schema;
mod ui;

use std::io;

use anyhow::Result;
use clap::Parser;
use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use rust_i18n::{i18n, t};
use zeroize::Zeroizing;

i18n!("locales", fallback = "en");

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(about = "OpenLDAP TUI editor")]
struct Args {
    #[arg(long, default_value = "ldapi://%2fvar%2frun%2fslapd%2fldapi")]
    uri: String,

    /// Bind DN for simple bind (ldap:// / ldaps://). Omit for SASL EXTERNAL (ldapi://) or anonymous.
    #[arg(long)]
    bind_dn: Option<String>,

    /// Base DN (search suffix). If omitted and multiple naming contexts exist, an interactive selection is shown.
    #[arg(short = 'b', long)]
    base_dn: Option<String>,
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    init_locale();

    let args = Args::parse();
    let password = resolve_password(&args)?;

    // panic hook: restore terminal before printing
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stderr(), LeaveAlternateScreen);
        original_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = app::App::init(
        &args.uri,
        args.bind_dn.as_deref(),
        password.as_ref(),
        args.base_dn.as_deref(),
    )
    .await;
    let result = ui::run(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn resolve_password(args: &Args) -> Result<Option<Zeroizing<String>>> {
    match &args.bind_dn {
        Some(_) => Ok(Some(Zeroizing::new(rpassword::prompt_password(
            t!("auth.password_prompt").to_string(),
        )?))),
        None => Ok(None),
    }
}

/// Detect locale from $LC_ALL / $LANG, normalize to a supported language code.
/// Falls back to "en" if no supported locale is detected.
fn init_locale() {
    let lang = sys_locale::get_locale().unwrap_or_default();
    let code = lang.split(['_', '-', '.']).next().unwrap_or("");
    let supported = match code {
        "ja" => "ja",
        _ => "en",
    };
    rust_i18n::set_locale(supported);
}
