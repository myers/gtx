use eyre::Result;

/// Launch $VISUAL or $EDITOR to edit text. Returns the edited content.
/// Falls back to `vi` if neither env var is set.
pub fn edit_body(initial: &str) -> Result<String> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let mut tmp = tempfile::Builder::new()
        .suffix(".md")
        .tempfile()?;
    std::io::Write::write_all(&mut tmp, initial.as_bytes())?;
    let path = tmp.path().to_path_buf();

    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| eyre::eyre!("Failed to launch editor '{editor}': {e}"))?;

    if !status.success() {
        eyre::bail!("Editor exited with non-zero status");
    }

    Ok(std::fs::read_to_string(&path)?.trim().to_string())
}
