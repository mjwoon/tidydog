/// organizer.rs — parse ORGANIZER.md rules and derive proposed_dest.
///
/// Rule format: `<conditions> -> <dest>`
/// Conditions are space-separated clauses (AND logic).
/// Within a clause, comma-separated values are OR logic.
///
/// Supported clause types:
///   tag:<value>[,<value>...]   – topic_tags contains any of the values (case-insensitive)
///   lang:<value>[,<value>...]  – language equals any of the values
///   ext:<value>[,<value>...]   – file extension equals any of the values (case-insensitive)
///   name:<value>[,<value>...]  – filename contains any of the values (case-insensitive)

#[derive(Debug, PartialEq, Clone)]
pub enum Condition {
    Tag(Vec<String>),
    Lang(Vec<String>),
    Ext(Vec<String>),
    Name(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct OrganizerRule {
    pub conditions: Vec<Condition>,
    pub dest: String,
}

pub struct FileAttrs<'a> {
    pub filename: &'a str,
    pub ext: &'a str,         // without dot, lowercase
    pub topic_tags: &'a [String],
    pub language: &'a str,
}

/// Parse ORGANIZER.md content into a list of rules.
pub fn parse_rules(organizer_md: &str) -> Vec<OrganizerRule> {
    let mut rules = Vec::new();

    for line in organizer_md.lines() {
        let trimmed = line.trim();

        // Skip blank lines and comments.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Split on ` -> ` to separate conditions from destination.
        let Some((cond_part, dest_part)) = trimmed.split_once(" -> ") else {
            continue;
        };

        let dest = dest_part.trim().to_string();
        if dest.is_empty() {
            continue;
        }

        let mut conditions = Vec::new();
        let mut valid = true;

        for clause in cond_part.split_whitespace() {
            let Some((key, values_str)) = clause.split_once(':') else {
                valid = false;
                break;
            };

            let values: Vec<String> = values_str
                .split(',')
                .map(|v| v.to_lowercase())
                .filter(|v| !v.is_empty())
                .collect();

            if values.is_empty() {
                valid = false;
                break;
            }

            let condition = match key {
                "tag"  => Condition::Tag(values),
                "lang" => Condition::Lang(values),
                "ext"  => Condition::Ext(values),
                "name" => Condition::Name(values),
                _ => {
                    valid = false;
                    break;
                }
            };

            conditions.push(condition);
        }

        if valid && !conditions.is_empty() {
            rules.push(OrganizerRule { conditions, dest });
        }
    }

    rules
}

/// Evaluate a single condition against the given file attributes.
fn condition_matches(cond: &Condition, attrs: &FileAttrs) -> bool {
    match cond {
        Condition::Tag(values) => {
            let tags_lower: Vec<String> = attrs
                .topic_tags
                .iter()
                .map(|t| t.to_lowercase())
                .collect();
            values.iter().any(|v| tags_lower.contains(v))
        }
        Condition::Lang(values) => {
            let lang_lower = attrs.language.to_lowercase();
            values.iter().any(|v| v == &lang_lower)
        }
        Condition::Ext(values) => {
            let ext_lower = attrs.ext.to_lowercase();
            values.iter().any(|v| v == &ext_lower)
        }
        Condition::Name(values) => {
            let name_lower = attrs.filename.to_lowercase();
            values.iter().any(|v| name_lower.contains(v.as_str()))
        }
    }
}

/// Return the dest of the first rule whose ALL conditions match, or None.
pub fn derive_dest(rules: &[OrganizerRule], attrs: &FileAttrs) -> Option<String> {
    for rule in rules {
        if rule.conditions.iter().all(|c| condition_matches(c, attrs)) {
            return Some(rule.dest.clone());
        }
    }
    None
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_dest_basic() {
        let rules = parse_rules("tag:세금,tax -> 문서/세금\nlang:ko -> 문서/기타\n");
        let attrs = FileAttrs {
            filename: "invoice.pdf",
            ext: "pdf",
            topic_tags: &["세금".to_string()],
            language: "ko",
        };
        assert_eq!(derive_dest(&rules, &attrs), Some("문서/세금".to_string()));

        let attrs2 = FileAttrs {
            filename: "notes.txt",
            ext: "txt",
            topic_tags: &[],
            language: "ko",
        };
        assert_eq!(derive_dest(&rules, &attrs2), Some("문서/기타".to_string()));

        let attrs3 = FileAttrs {
            filename: "doc.pdf",
            ext: "pdf",
            topic_tags: &[],
            language: "en",
        };
        assert_eq!(derive_dest(&rules, &attrs3), None);
    }

    #[test]
    fn test_parse_skips_comments_and_blank() {
        let md = "
# This is a comment

# Another comment
tag:foo -> bar
";
        let rules = parse_rules(md);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].dest, "bar");
    }

    #[test]
    fn test_multi_clause_and_logic() {
        let rules = parse_rules("lang:ko ext:pdf -> 문서/PDF\n");
        // Both lang=ko AND ext=pdf must match.
        let attrs_match = FileAttrs {
            filename: "doc.pdf",
            ext: "pdf",
            topic_tags: &[],
            language: "ko",
        };
        assert_eq!(derive_dest(&rules, &attrs_match), Some("문서/PDF".to_string()));

        let attrs_no_ext = FileAttrs {
            filename: "doc.txt",
            ext: "txt",
            topic_tags: &[],
            language: "ko",
        };
        assert_eq!(derive_dest(&rules, &attrs_no_ext), None);
    }

    #[test]
    fn test_name_clause() {
        let rules = parse_rules("name:영수증,receipt -> 문서/영수증\n");
        let attrs = FileAttrs {
            filename: "receipt_2024.pdf",
            ext: "pdf",
            topic_tags: &[],
            language: "en",
        };
        assert_eq!(derive_dest(&rules, &attrs), Some("문서/영수증".to_string()));

        let attrs_ko = FileAttrs {
            filename: "영수증001.pdf",
            ext: "pdf",
            topic_tags: &[],
            language: "ko",
        };
        assert_eq!(derive_dest(&rules, &attrs_ko), Some("문서/영수증".to_string()));
    }

    #[test]
    fn test_first_match_wins() {
        let rules = parse_rules("tag:세금 -> 문서/세금\nlang:ko -> 문서/기타\n");
        let attrs = FileAttrs {
            filename: "세금.pdf",
            ext: "pdf",
            topic_tags: &["세금".to_string()],
            language: "ko",
        };
        // First rule (tag:세금) matches before the lang:ko rule.
        assert_eq!(derive_dest(&rules, &attrs), Some("문서/세금".to_string()));
    }

    #[test]
    fn test_tag_case_insensitive() {
        let rules = parse_rules("tag:Tax -> TaxDocs\n");
        let attrs = FileAttrs {
            filename: "tax_2024.pdf",
            ext: "pdf",
            topic_tags: &["TAX".to_string()],
            language: "en",
        };
        assert_eq!(derive_dest(&rules, &attrs), Some("TaxDocs".to_string()));
    }
}
