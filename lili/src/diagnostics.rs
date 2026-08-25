use tracing_subscriber::EnvFilter;

pub fn init() {
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();
}

pub fn info_with_counts(
    component: &'static str,
    operation: &'static str,
    code: &'static str,
    primary_count: usize,
    diagnostic_count: usize,
) {
    tracing::info!(
        component,
        operation,
        outcome = "success",
        code,
        primary_count,
        diagnostic_count
    );
}

pub fn info(component: &'static str, operation: &'static str, code: &'static str) {
    tracing::info!(component, operation, outcome = "success", code);
}

pub fn warn(component: &'static str, operation: &'static str, code: &'static str) {
    tracing::warn!(component, operation, outcome = "failure", code);
}

pub fn error(component: &'static str, operation: &'static str, code: &'static str) {
    tracing::error!(component, operation, outcome = "failure", code);
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    #[test]
    fn production_modules_cannot_emit_unreviewed_tracing_fields() {
        let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in fs::read_dir(source_dir).unwrap() {
            let path = entry.unwrap().path();
            if path
                .file_name()
                .is_some_and(|name| name == "diagnostics.rs")
            {
                continue;
            }
            if path.extension().is_some_and(|extension| extension == "rs") {
                let source = fs::read_to_string(&path).unwrap();
                assert!(
                    !source.contains("tracing::"),
                    "unreviewed tracing call in {}",
                    path.display()
                );
            }
        }
    }
}
