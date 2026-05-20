use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, ValueEnum)]
enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

#[derive(Parser)]
#[command(name = "apitool", about = "A simple HTTP request tool", version)]
struct Cli {
    /// HTTP method
    method: Method,

    /// Target URL
    url: String,

    /// Headers in KEY:VALUE format (repeatable)
    #[arg(short = 'H', long = "header", value_name = "KEY:VALUE")]
    headers: Vec<String>,

    /// Query parameters in KEY=VALUE format (repeatable)
    #[arg(short = 'q', long = "query", value_name = "KEY=VALUE")]
    query: Vec<String>,

    /// Request body (JSON string)
    #[arg(short = 'd', long = "data", value_name = "BODY")]
    data: Option<String>,

    /// Read request body from file
    #[arg(short = 'f', long = "file", value_name = "FILE")]
    file: Option<std::path::PathBuf>,

    /// Show response headers
    #[arg(short = 'i', long = "include-headers")]
    include_headers: bool,
}

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

fn status_color(code: u16) -> colored::ColoredString {
    let s = code.to_string();
    match code {
        200..=299 => s.bright_green().bold(),
        300..=399 => s.bright_yellow().bold(),
        400..=499 => s.bright_red().bold(),
        _ => s.red().bold(),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Build headers map
    let mut header_map: HashMap<String, String> = HashMap::new();
    for raw in &cli.headers {
        let (k, v) = parse_header(raw)?;
        header_map.insert(k, v);
    }

    // Build query params
    let mut query_params: Vec<(String, String)> = Vec::new();
    for raw in &cli.query {
        query_params.push(parse_query(raw)?);
    }

    // Resolve body
    let body: Option<String> = match (&cli.data, &cli.file) {
        (Some(_), Some(_)) => bail!("--data and --file are mutually exclusive"),
        (Some(d), None) => Some(d.clone()),
        (None, Some(path)) => Some(
            std::fs::read_to_string(path)
                .with_context(|| format!("could not read body file `{}`", path.display()))?,
        ),
        (None, None) => None,
    };

    // Validate body is valid JSON when present
    if let Some(ref raw) = body {
        serde_json::from_str::<serde_json::Value>(raw)
            .with_context(|| "request body is not valid JSON")?;
    }

    // Set up progress spinner
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    spinner.set_message(format!(
        "{} {}",
        format!("{:?}", cli.method).to_uppercase().bright_cyan(),
        cli.url.dimmed()
    ));
    spinner.enable_steady_tick(Duration::from_millis(80));

    // Build and send request
    let client = reqwest::blocking::Client::new();

    let mut req = match cli.method {
        Method::Get => client.get(&cli.url),
        Method::Post => client.post(&cli.url),
        Method::Put => client.put(&cli.url),
        Method::Patch => client.patch(&cli.url),
        Method::Delete => client.delete(&cli.url),
    };

    for (k, v) in &header_map {
        req = req.header(k, v);
    }

    if !query_params.is_empty() {
        req = req.query(&query_params);
    }

    if let Some(ref b) = body {
        req = req
            .header("Content-Type", "application/json")
            .body(b.clone());
    }

    let response = req.send().with_context(|| format!("request to `{}` failed", cli.url))?;

    spinner.finish_and_clear();

    // Display status line
    let status = response.status();
    let version = format!("{:?}", response.version());
    println!(
        "{} {} {}",
        version.dimmed(),
        status_color(status.as_u16()),
        status.canonical_reason().unwrap_or("").dimmed()
    );

    // Optionally display response headers
    if cli.include_headers {
        println!("{}", "── Headers ──────────────────────────────".dimmed());
        for (name, value) in response.headers() {
            println!(
                "  {}: {}",
                name.as_str().bright_blue(),
                value.to_str().unwrap_or("<binary>").dimmed()
            );
        }
        println!("{}", "─────────────────────────────────────────".dimmed());
    }

    // Read body
    let body_text = response.text().context("failed to read response body")?;

    println!();

    // Attempt JSON pretty-print; fall back to raw text
    match serde_json::from_str::<serde_json::Value>(&body_text) {
        Ok(json) => {
            let pretty = serde_json::to_string_pretty(&json)?;
            println!("{}", colorize_json(&pretty));
        }
        Err(_) => {
            println!("{}", body_text);
        }
    }

    Ok(())
}

/// Minimal JSON syntax highlighter operating line-by-line.
fn colorize_json(pretty: &str) -> String {
    pretty
        .lines()
        .map(|line| {
            // Key: "foo":
            if let Some(colon_pos) = line.find("\": ") {
                let before = &line[..colon_pos + 2]; // up to and including `"`
                let after = &line[colon_pos + 2..];  // value portion

                let colored_key = before.bright_blue().to_string();
                let colored_val = colorize_value(after.trim_end_matches(','));
                let trailing_comma = if after.trim_end().ends_with(',') { "," } else { "" };
                format!("{colored_key} {colored_val}{trailing_comma}")
            } else {
                // Structural line (braces, brackets, bare values)
                colorize_value(line).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn colorize_value(s: &str) -> colored::ColoredString {
    let trimmed = s.trim();
    if trimmed.starts_with('"') {
        trimmed.bright_green()
    } else if trimmed == "true" || trimmed == "false" {
        trimmed.bright_yellow()
    } else if trimmed == "null" {
        trimmed.bright_magenta()
    } else if trimmed.parse::<f64>().is_ok() {
        trimmed.bright_cyan()
    } else {
        trimmed.white()
    }
}
