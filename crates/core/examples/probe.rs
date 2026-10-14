//! Live smoke test: `cargo run -p lyrixearch-core --example probe -- "Artist" "Title"`.
//! Prints per-provider status, match info and text lengths (never the lyrics themselves).

use lyrixearch_core::{Config, Engine, Query};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (artist, title) = match args.as_slice() {
        [a, t] => (a.clone(), t.clone()),
        _ => ("Queen".into(), "Bohemian Rhapsody".into()),
    };
    let cfg = Config {
        providers: lyrixearch_core::providers::all(&Config::plugins_dir()).iter().map(|p| p.id().to_string()).collect(),
        penalty_max_failures: 1000,
        ..Config::default()
    };

    for id in cfg.providers.clone() {
        let mut one = cfg.clone();
        one.providers = vec![id.clone()];
        let engine = Engine::new(one)?;
        let q = Query { artist: artist.clone(), title: title.clone(), duration_secs: None, ..Default::default() };
        let t = std::time::Instant::now();
        let out = engine.find(&q, |_, _| {}).await;
        match out {
            lyrixearch_core::Outcome::Found(l) => {
                if std::env::var_os("PROBE_TEXT").is_some() {
                    println!("---\n{}\n---", l.plain.as_deref().unwrap_or(""));
                }
                println!(
                    "{id:12} FOUND  {:>5.0?} score={:.2} plain={} synced={} match=\"{} - {}\"",
                    t.elapsed(),
                    l.score,
                    l.plain.as_deref().map(str::len).unwrap_or(0),
                    l.synced.as_deref().map(str::len).unwrap_or(0),
                    l.matched_artist,
                    l.matched_title
                )
            }
            lyrixearch_core::Outcome::NotFound(notes) => println!("{id:12} none   {:>5.0?} {}", t.elapsed(), notes.join("; ")),
        }
    }
    Ok(())
}
