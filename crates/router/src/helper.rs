fn main() {
    let result = (|| -> anyhow::Result<()> {
        let mut args = std::env::args_os().skip(1);
        let path = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("Missing scoped hosts plan"))?;
        let hash = args
            .next()
            .ok_or_else(|| anyhow::anyhow!("Missing hosts plan digest"))?;
        if args.next().is_some() {
            anyhow::bail!("Unexpected helper arguments");
        }
        dockon_router::permissions::apply_hosts_plan(
            std::path::Path::new(&path),
            hash.to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid plan digest"))?,
        )
    })();
    if let Err(error) = result {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
