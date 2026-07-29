//! Deterministic, human-readable slugs for entry IDs.

/// Lowercase, collapse any run of non-alphanumeric characters to a single `-`,
/// and trim leading/trailing dashes. `"Billing Portal"` -> `"billing-portal"`.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in s.chars() {
        if c.is_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.extend(c.to_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugs_are_clean() {
        assert_eq!(slugify("Acme Corp"), "acme-corp");
        assert_eq!(slugify("Billing Portal"), "billing-portal");
        assert_eq!(slugify("  R&D / Research  "), "r-d-research");
        assert_eq!(slugify("Development"), "development");
    }
}
