use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use snellar_core::product;

use crate::api;
use crate::client::call;
use crate::render;

const USAGE: &str = "usage: snellar <command>

  now                    what is playing
  queue                  the queue
  search <query>         search songs
  play [query]           play the best match (no query: resume)
  next | prev | toggle   transport
  vol <0-100>            volume
  like                   toggle like on the current track
  ctl <method> [json]    any API method, JSON in and out (snellar ctl --methods)
  ctl reports            saved bug reports, newest first
  ctl diag               DNS, TCP, TLS and HTTP to each host Snellar needs
  mcp                    MCP server on stdio
  --version              the version and the commit of this build
  --update-key           the key updates must be signed with, in hex, or none

Without a command snellar starts the app.";

const COMMANDS: [&str; 11] = ["ctl", "mcp", "now", "queue", "search", "play", "next", "prev", "toggle", "vol", "like"];

pub fn run(args: &[String]) -> Option<i32> {
    let first = args.first()?.as_str();
    if first == "--help" || first == "-h" || first == "help" {
        println!("{USAGE}");
        return Some(0);
    }
    if first == "--version" {
        println!("{} {} ({})", product::NAME, product::VERSION, crate::stamp::commit());
        return Some(0);
    }
    if first == "--update-key" {
        println!("{}", crate::update::compiled_key_hex());
        return Some(0);
    }
    if !COMMANDS.contains(&first) {
        return None;
    }
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let outcome = runtime.block_on(async {
        if first == "mcp" {
            return crate::mcp::serve().await.map(|_| String::new());
        }
        command(args).await
    });
    Some(match outcome {
        Ok(text) => {
            if !text.is_empty() {
                println!("{text}");
            }
            0
        }
        Err(e) => {
            eprintln!("{}: {e:#}", product::ID);
            1
        }
    })
}

pub async fn command(args: &[String]) -> Result<String> {
    let rest = args[1..].join(" ");
    match args[0].as_str() {
        "ctl" => ctl(&args[1..]).await,
        "now" => Ok(render::now(&call("now", &Value::Null).await?)),
        "queue" => Ok(render::queue(&call("queue", &Value::Null).await?, 40)),
        "search" => {
            let tracks = songs(&rest).await?;
            Ok(if tracks.is_empty() { "No songs found.".into() } else { render::results(&tracks) })
        }
        "play" if rest.is_empty() => {
            call("toggle", &Value::Null).await?;
            Ok("ok".into())
        }
        "play" => {
            let tracks = songs(&rest).await?;
            let Some(top) = tracks.first() else { bail!("no songs found for `{rest}`") };
            let line = render::track(top);
            call("play", &json!({ "tracks": tracks, "index": 0 })).await?;
            Ok(format!("Playing {line}"))
        }
        name @ ("next" | "prev" | "toggle") => {
            call(name, &Value::Null).await?;
            Ok("ok".into())
        }
        "vol" => {
            let value: f64 = rest.parse().context("vol takes a number from 0 to 100")?;
            call("volume", &json!({ "value": value })).await?;
            Ok("ok".into())
        }
        "like" => {
            let now = call("now", &Value::Null).await?;
            if now["current"].is_null() {
                bail!("nothing is playing");
            }
            let liked = call("like", &json!({ "track": now["current"] })).await?;
            Ok(if liked == json!(true) { "liked".into() } else { "unliked".into() })
        }
        other => bail!("unknown command `{other}`"),
    }
}

async fn songs(query: &str) -> Result<Vec<Value>> {
    let found = call("search", &json!({ "q": query, "filter": "songs" })).await?;
    Ok(found["tracks"].as_array().cloned().unwrap_or_default())
}

async fn ctl(args: &[String]) -> Result<String> {
    match args.first().map(String::as_str) {
        None => bail!("usage: snellar ctl <method> [json] | --methods | --docs"),
        Some("--docs") => Ok(api::markdown()),
        Some("diag") => Ok(crate::diag::report().await),
        Some("--methods") => Ok(api::METHODS.iter().map(|m| format!("{:<20} {}", m.name, m.args)).collect::<Vec<_>>().join("\n")),
        Some(method) => {
            let body = args[1..].join(" ");
            let params = if body.trim().is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&body).context("the argument must be a JSON object")?
            };
            let value = call(method, &params).await?;
            Ok(match method {
                "reports" => render::reports(&value),
                "importPlaylist" => render::import(&value),
                _ => value.to_string(),
            })
        }
    }
}
