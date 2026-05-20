use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ── Data types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RequestRecord {
    method: String,
    url: String,
    headers: HashMap<String, String>,
    query: Vec<(String, String)>,
    body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResponseRecord {
    status: u16,
    body: String,
    duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryEntry {
    id: u64,
    timestamp: String,
    alias: Option<String>,
    request: RequestRecord,
    response: ResponseRecord,
}

type Aliases = HashMap<String, RequestRecord>;

// ── Storage ────────────────────────────────────────────────────────────────

fn apitool_dir() -> Result<std::path::PathBuf> {
    let home = std::env::var("HOME").context("HOME env var not set")?;
    let dir = std::path::PathBuf::from(home).join(".apitool");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn load_history() -> Result<Vec<HistoryEntry>> {
    let path = apitool_dir()?.join("history.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn save_history(history: &[HistoryEntry]) -> Result<()> {
    let path = apitool_dir()?.join("history.json");
    std::fs::write(path, serde_json::to_string_pretty(history)?)?;
    Ok(())
}

fn append_history(entry: HistoryEntry) -> Result<()> {
    let mut history = load_history()?;
    history.push(entry);
    // Cap at 1000 entries
    if history.len() > 1000 {
        history.drain(..history.len() - 1000);
    }
    save_history(&history)
}

fn load_aliases() -> Result<Aliases> {
    let path = apitool_dir()?.join("aliases.json");
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

fn save_aliases(aliases: &Aliases) -> Result<()> {
    let path = apitool_dir()?.join("aliases.json");
    std::fs::write(path, serde_json::to_string_pretty(aliases)?)?;
    Ok(())
}

// ── CLI ────────────────────────────────────────────────────────────────────

/// Shared options for all request commands.
#[derive(Args, Clone)]
struct RequestArgs {
    /// Target URL
    url: String,

    /// Header in KEY:VALUE format (repeatable)
    #[arg(short = 'H', long = "header", value_name = "KEY:VALUE")]
    headers: Vec<String>,

    /// Query parameter in KEY=VALUE format (repeatable)
    #[arg(short = 'q', long = "query", value_name = "KEY=VALUE")]
    query: Vec<String>,

    /// JSON request body
    #[arg(short = 'd', long = "data", value_name = "BODY")]
    data: Option<String>,

    /// Read JSON request body from file
    #[arg(short = 'f', long = "file", value_name = "FILE")]
    file: Option<std::path::PathBuf>,

    /// Print response headers
    #[arg(short = 'i', long = "include-headers")]
    include_headers: bool,

    /// Save this request as a named alias
    #[arg(long = "save-as", value_name = "NAME")]
    save_as: Option<String>,
}

#[derive(Parser)]
#[command(
    name = "apitool",
    about = "A curl-like HTTP client with request history and saved aliases",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a GET request
    Get {
        #[command(flatten)]
        args: RequestArgs,
    },
    /// Send a POST request
    Post {
        #[command(flatten)]
        args: RequestArgs,
    },
    /// Send a PUT request
    Put {
        #[command(flatten)]
        args: RequestArgs,
    },
    /// Send a PATCH request
    Patch {
        #[command(flatten)]
        args: RequestArgs,
    },
    /// Send a DELETE request
    Delete {
        #[command(flatten)]
        args: RequestArgs,
    },
    /// Run a saved alias (with optional overrides)
    Run {
        /// Name of the alias
        name: String,
        /// Append or override query parameters
        #[arg(short = 'q', long = "query", value_name = "KEY=VALUE")]
        query: Vec<String>,
        /// Append or override headers
        #[arg(short = 'H', long = "header", value_name = "KEY:VALUE")]
        headers: Vec<String>,
        /// Override request body
        #[arg(short = 'd', long = "data", value_name = "BODY")]
        data: Option<String>,
        /// Print response headers
        #[arg(short = 'i', long = "include-headers")]
        include_headers: bool,
    },
    /// Manage saved aliases
    Alias {
        #[command(subcommand)]
        action: AliasAction,
    },
    /// View request history
    History {
        /// Number of entries to display
        #[arg(short = 'n', long, default_value = "20")]
        limit: usize,
        /// Erase all history
        #[arg(long)]
        clear: bool,
    },
}

#[derive(Subcommand)]
enum AliasAction {
    /// List all saved aliases
    List,
    /// Show the full configuration of an alias
    Show {
        /// Alias name
        name: String,
    },
    /// Delete a saved alias
    Delete {
        /// Alias name
        name: String,
    },
}

// ── Parsing helpers ────────────────────────────────────────────────────────

fn parse_header(raw: &str) -> Result<(String, String)> {
    let (k, v) = raw
        .split_once(':')
        .with_context(|| format!("invalid header `{raw}` — expected KEY:VALUE"))?;
    Ok((k.trim().to_string(), v.trim().to_string()))
}

fn parse_query(raw: &str) -> Result<(String, String)> {
    let (k, v) = raw
        .split_once('=')
        .with_context(|| format!("invalid query param `{raw}` — expected KEY=VALUE"))?;
    Ok((k.trim().to_string(), v.trim().to_string()))
}

fn resolve_body(data: Option<&str>, file: Option<&std::path::PathBuf>) -> Result<Option<String>> {
    match (data, file) {
        (Some(_), Some(_)) => bail!("--data and --file are mutually exclusive"),
        (Some(d), None) => {
            serde_json::from_str::<serde_json::Value>(d).context("request body is not valid JSON")?;
            Ok(Some(d.to_string()))
        }
        (None, Some(path)) => {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("could not read `{}`", path.display()))?;
            serde_json::from_str::<serde_json::Value>(&raw)
                .context("body file is not valid JSON")?;
            Ok(Some(raw))
        }
        (None, None) => Ok(None),
    }
}

fn build_request(method: &str, args: &RequestArgs) -> Result<RequestRecord> {
    let mut headers: HashMap<String, String> = HashMap::new();
    for raw in &args.headers {
        let (k, v) = parse_header(raw)?;
        headers.insert(k, v);
    }
    let mut query = Vec::new();
    for raw in &args.query {
        query.push(parse_query(raw)?);
    }
    let body = resolve_body(args.data.as_deref(), args.file.as_ref())?;
    Ok(RequestRecord {
        method: method.to_string(),
        url: args.url.clone(),
        headers,
        query,
        body,
    })
}

// ── HTTP ───────────────────────────────────────────────────────────────────

fn send_request(record: &RequestRecord, include_headers: bool) -> Result<ResponseRecord> {
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    spinner.set_message(format!(
        "{} {}",
        record.method.as_str().bright_cyan().bold(),
        record.url.as_str().dimmed(),
    ));
    spinner.enable_steady_tick(Duration::from_millis(80));

    let client = reqwest::blocking::Client::new();
    let method = reqwest::Method::from_bytes(record.method.as_bytes())
        .with_context(|| format!("unknown method `{}`", record.method))?;

    let mut req = client.request(method, &record.url);
    for (k, v) in &record.headers {
        req = req.header(k, v);
    }
    if !record.query.is_empty() {
        req = req.query(&record.query);
    }
    if let Some(ref b) = record.body {
        req = req.header("Content-Type", "application/json").body(b.clone());
    }

    let t0 = Instant::now();
    let resp = req
        .send()
        .with_context(|| format!("request to `{}` failed", record.url))?;
    let duration_ms = t0.elapsed().as_millis() as u64;

    spinner.finish_and_clear();

    let status = resp.status();
    println!(
        "{}  {}  ({} ms)",
        status_color(status.as_u16()),
        status.canonical_reason().unwrap_or("").dimmed(),
        duration_ms.to_string().bright_white(),
    );

    if include_headers {
        println!("{}", "── Response Headers ──────────────────────".dimmed());
        for (name, value) in resp.headers() {
            println!(
                "  {}: {}",
                name.as_str().bright_blue(),
                value.to_str().unwrap_or("<binary>").dimmed(),
            );
        }
        println!("{}", "──────────────────────────────────────────".dimmed());
    }

    let body = resp.text().context("failed to read response body")?;
    println!();
    print_body(&body)?;

    Ok(ResponseRecord {
        status: status.as_u16(),
        body,
        duration_ms,
    })
}

fn print_body(body: &str) -> Result<()> {
    match serde_json::from_str::<serde_json::Value>(body) {
        Ok(json) => println!("{}", colorize_json(&serde_json::to_string_pretty(&json)?)),
        Err(_) => println!("{}", body),
    }
    Ok(())
}

fn record_to_history(
    record: RequestRecord,
    response: ResponseRecord,
    alias: Option<String>,
) -> Result<()> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    append_history(HistoryEntry {
        id: ms,
        timestamp: format_timestamp(ms / 1000),
        alias,
        request: record,
        response,
    })
}

fn execute_request(method: &str, args: &RequestArgs) -> Result<()> {
    let record = build_request(method, args)?;
    let response = send_request(&record, args.include_headers)?;

    if let Some(ref name) = args.save_as {
        let mut aliases = load_aliases()?;
        aliases.insert(name.clone(), record.clone());
        save_aliases(&aliases)?;
        println!("\n{} saved alias `{}`", "✓".bright_green(), name.bright_white().bold());
    }

    record_to_history(record, response, args.save_as.clone())
}

// ── Display helpers ────────────────────────────────────────────────────────

fn status_color(code: u16) -> colored::ColoredString {
    let s = code.to_string();
    match code {
        200..=299 => s.bright_green().bold(),
        300..=399 => s.bright_yellow().bold(),
        400..=499 => s.bright_red().bold(),
        _ => s.red().bold(),
    }
}

fn colorize_json(pretty: &str) -> String {
    pretty
        .lines()
        .map(|line| {
            if let Some(pos) = line.find("\": ") {
                // Line with a key: `  "key": value[,]`
                let key_part = &line[..pos + 2];
                let val_part = &line[pos + 2..];
                let trailing = val_part.trim_end().ends_with(',');
                let val = val_part.trim_end_matches(',').trim();
                format!(
                    "{} {}{}",
                    key_part.bright_blue(),
                    colorize_value(val),
                    if trailing { "," } else { "" },
                )
            } else {
                // Structural line: `{`, `}`, `[`, `]`
                let trimmed = line.trim_start();
                let indent = &line[..line.len() - trimmed.len()];
                format!("{}{}", indent, colorize_value(trimmed))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn colorize_value(s: &str) -> colored::ColoredString {
    if s.starts_with('"') {
        s.bright_green()
    } else if s == "true" || s == "false" {
        s.bright_yellow()
    } else if s == "null" {
        s.bright_magenta()
    } else if s.trim_end_matches(',').parse::<f64>().is_ok() {
        s.bright_cyan()
    } else {
        s.white()
    }
}

fn format_timestamp(secs: u64) -> String {
    let sec = secs % 60;
    let min = (secs / 60) % 60;
    let hr = (secs / 3600) % 24;
    let mut days = secs / 86400;
    let mut year = 1970u64;
    loop {
        let y_days = if is_leap(year) { 366 } else { 365 };
        if days < y_days {
            break;
        }
        days -= y_days;
        year += 1;
    }
    let month_lens = [31u64, if is_leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u64;
    for &ml in &month_lens {
        if days < ml {
            break;
        }
        days -= ml;
        month += 1;
    }
    let day = days + 1;
    format!("{year:04}-{month:02}-{day:02} {hr:02}:{min:02}:{sec:02}")
}

fn is_leap(y: u64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Get { args } => execute_request("GET", &args),
        Commands::Post { args } => execute_request("POST", &args),
        Commands::Put { args } => execute_request("PUT", &args),
        Commands::Patch { args } => execute_request("PATCH", &args),
        Commands::Delete { args } => execute_request("DELETE", &args),

        Commands::Run { name, query, headers, data, include_headers } => {
            let aliases = load_aliases()?;
            let base = aliases
                .get(&name)
                .with_context(|| format!("alias `{name}` not found — run `apitool alias list` to see saved aliases"))?
                .clone();

            let mut merged_headers = base.headers.clone();
            for raw in &headers {
                let (k, v) = parse_header(raw)?;
                merged_headers.insert(k, v);
            }
            let mut merged_query = base.query.clone();
            for raw in &query {
                merged_query.push(parse_query(raw)?);
            }
            let body = if let Some(ref d) = data {
                serde_json::from_str::<serde_json::Value>(d)
                    .context("request body is not valid JSON")?;
                data
            } else {
                base.body
            };

            let record = RequestRecord {
                method: base.method,
                url: base.url,
                headers: merged_headers,
                query: merged_query,
                body,
            };

            println!("{} {}", "→".dimmed(), name.bright_cyan().bold());
            let response = send_request(&record, include_headers)?;
            record_to_history(record, response, Some(name))
        }

        Commands::Alias { action } => match action {
            AliasAction::List => {
                let aliases = load_aliases()?;
                if aliases.is_empty() {
                    println!("{}", "No aliases saved yet.  Use --save-as NAME on any request.".dimmed());
                    return Ok(());
                }
                println!("{}", "Saved aliases:".bright_white().bold());
                let mut names: Vec<_> = aliases.keys().collect();
                names.sort();
                for name in names {
                    let r = &aliases[name];
                    println!(
                        "  {}  {} {}",
                        name.bright_cyan(),
                        r.method.bright_blue(),
                        r.url.dimmed(),
                    );
                }
                Ok(())
            }

            AliasAction::Show { name } => {
                let aliases = load_aliases()?;
                let r = aliases
                    .get(&name)
                    .with_context(|| format!("alias `{name}` not found"))?;

                println!("{} {}", "alias:".dimmed(), name.bright_white().bold());
                println!("  {}  {}", r.method.bright_cyan().bold(), r.url.bright_blue());
                if !r.headers.is_empty() {
                    println!("  {}:", "headers".dimmed());
                    for (k, v) in &r.headers {
                        println!("    {}: {}", k.bright_blue(), v.dimmed());
                    }
                }
                if !r.query.is_empty() {
                    println!("  {}:", "query".dimmed());
                    for (k, v) in &r.query {
                        println!("    {}={}", k.bright_blue(), v.dimmed());
                    }
                }
                if let Some(ref body) = r.body {
                    println!("  {}:", "body".dimmed());
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
                        let pretty = serde_json::to_string_pretty(&json)?;
                        for line in pretty.lines() {
                            println!("    {line}");
                        }
                    } else {
                        println!("    {}", body.dimmed());
                    }
                }
                Ok(())
            }

            AliasAction::Delete { name } => {
                let mut aliases = load_aliases()?;
                if aliases.remove(&name).is_none() {
                    bail!("alias `{name}` not found");
                }
                save_aliases(&aliases)?;
                println!("{} deleted alias `{}`", "✓".bright_green(), name);
                Ok(())
            }
        },

        Commands::History { limit, clear } => {
            if clear {
                save_history(&[])?;
                println!("{} history cleared", "✓".bright_green());
                return Ok(());
            }
            let history = load_history()?;
            if history.is_empty() {
                println!("{}", "No history yet.".dimmed());
                return Ok(());
            }
            let total = history.len();
            println!(
                "  {:>4}  {:<19}  {:<6}  {}  {:>6}  {}",
                "#".dimmed(),
                "time (UTC)".dimmed(),
                "method".dimmed(),
                "st".dimmed(),
                "ms".dimmed(),
                "url".dimmed(),
            );
            println!("{}", "─".repeat(74).dimmed());
            for (i, entry) in history.iter().rev().take(limit).enumerate() {
                let num = total - i;
                let alias_tag = entry
                    .alias
                    .as_deref()
                    .map(|a| format!("  [{}]", a.bright_cyan()))
                    .unwrap_or_default();
                println!(
                    "  {:>4}  {}  {:6}  {}  {:>6}  {}{}",
                    num.to_string().dimmed(),
                    entry.timestamp.dimmed(),
                    entry.request.method.bright_blue(),
                    status_color(entry.response.status),
                    entry.response.duration_ms.to_string().dimmed(),
                    entry.request.url,
                    alias_tag,
                );
            }
            Ok(())
        }
    }
}
