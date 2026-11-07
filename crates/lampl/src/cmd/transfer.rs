// SPDX-License-Identifier: Apache-2.0

use std::io::{Read, Write};
use std::path::PathBuf;

use clap::Args as ClapArgs;

use super::server;
use crate::import::{self, Source};

#[derive(ClapArgs)]
pub(crate) struct ExportArgs {
    #[arg(long)]
    project: Option<String>,
    /// Leave vectors out (they are recomputed on import)
    #[arg(long)]
    no_vectors: bool,
    /// Write to this file instead of stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub(crate) fn export(args: &ExportArgs) -> Result<(), String> {
    let client = server("export")?;
    let mut path = format!("/v1/export?vectors={}", !args.no_vectors);
    if let Some(p) = &args.project {
        path.push_str("&project=");
        path.push_str(
            &p.replace('%', "%25")
                .replace('&', "%26")
                .replace(' ', "%20"),
        );
    }
    let mut body = Vec::new();
    client
        .stream(&path)?
        .read_to_end(&mut body)
        .map_err(|e| format!("export stream broke: {e}; nothing was written"))?;
    let text = std::str::from_utf8(&body).map_err(|_| "export is not UTF-8")?;
    if !text
        .lines()
        .last()
        .is_some_and(|l| l.contains(r#""kind":"end""#))
    {
        return Err("export ended without an end marker; nothing was written".to_owned());
    }
    match &args.output {
        Some(p) => std::fs::write(p, &body).map_err(|e| format!("writing {}: {e}", p.display())),
        None => std::io::stdout()
            .write_all(&body)
            .map_err(|e| e.to_string()),
    }
}

#[derive(ClapArgs)]
pub(crate) struct ImportArgs {
    /// A lamplighter export or a memorygraph JSON export (`-` for stdin)
    #[arg(
        required_unless_present = "from_postgres",
        conflicts_with = "from_postgres"
    )]
    file: Option<PathBuf>,
    /// Read memorygraph's tables directly; URL from the flag or $LAMPL_FROM_POSTGRES_URL (keeps the password out of argv)
    #[arg(long, num_args = 0..=1, value_name = "URL")]
    from_postgres: Option<Option<String>>,
    /// Project given to memorygraph memories, which have none
    #[arg(long, default_value = "memorygraph")]
    project: String,
    /// Drop a lamplighter export's vectors and let the server re-embed
    #[arg(long)]
    reembed: bool,
    /// Convert and check, upload nothing
    #[arg(long)]
    dry_run: bool,
}

fn load(args: &ImportArgs) -> Result<Source, String> {
    if let Some(url) = &args.from_postgres {
        let url = url
            .clone()
            .or_else(|| std::env::var("LAMPL_FROM_POSTGRES_URL").ok())
            .ok_or("no database URL: pass --from-postgres URL or set LAMPL_FROM_POSTGRES_URL")?;
        return import::read_memory_pg(&url);
    }
    let path = args.file.as_ref().ok_or("no file given")?;
    let text = if path.as_os_str() == "-" {
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| e.to_string())?;
        s
    } else {
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?
    };
    import::parse_file(&text)
}

pub(crate) fn import(args: &ImportArgs) -> Result<(), String> {
    let mut bundle = match load(args)? {
        Source::Lamplighter(b) => b,
        Source::Memorygraph { memories, rels } => {
            import::from_memorygraph(&memories, &rels, &args.project)?
        }
    };
    let client = server("import")?;
    if bundle.has_vectors() {
        let health = client.get("/health")?;
        let theirs = bundle.model.clone().unwrap_or_default();
        let ours = (
            health["embed_model"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            health["embed_digest"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
        if theirs != ours && !args.reembed {
            return Err(format!(
                "embedding model mismatch: the export has {} ({}), the server uses {} ({}); pass --reembed to drop the vectors and re-embed",
                theirs.0, theirs.1, ours.0, ours.1
            ));
        }
        if args.reembed {
            bundle.strip_vectors();
        }
    }
    for (what, n) in &bundle.report {
        eprintln!("lampl: {what}: {n}");
    }
    if args.dry_run {
        println!(
            "{}",
            serde_json::json!({"dry_run": true, "memories": bundle.memories.len(), "links": bundle.links.len()})
        );
        return Ok(());
    }
    let out = client.post_ndjson("/v1/import", &bundle.ndjson()?)?;
    println!("{out}");
    Ok(())
}
