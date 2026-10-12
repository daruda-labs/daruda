//! Strict SKILL.md import; ordinary Markdown remains an exact instruction body.
use super::frontmatter::{SkillFrontmatter, parse_frontmatter, split_frontmatter};

#[derive(Clone, Debug)]
pub(crate) struct ImportedSkill {
    pub source: String,
    pub frontmatter: SkillFrontmatter,
    pub body: String,
}

pub(crate) fn parse(source: &str) -> Result<ImportedSkill, ()> {
    let (yaml, body) = split_frontmatter(source);
    if yaml.is_none()
        && source
            .trim_start_matches('\u{feff}')
            .lines()
            .next()
            .is_some_and(|l| l.trim() == "---")
    {
        return Err(());
    }
    let frontmatter = if let Some(yaml) = yaml {
        let value: yaml_serde::Value = yaml_serde::from_str(yaml).map_err(|_| ())?;
        if !matches!(
            value,
            yaml_serde::Value::Mapping(_) | yaml_serde::Value::Null
        ) {
            return Err(());
        }
        if let yaml_serde::Value::Mapping(map) = &value {
            for (key, value) in map {
                let key = key.as_str().ok_or(())?;
                let valid = match key {
                    "name" | "description" | "argument-hint" | "when_to_use" | "model"
                    | "context" | "agent" | "effort" | "shell" => value.is_string(),
                    "disable-model-invocation" | "user-invocable" => value.is_bool(),
                    "allowed-tools" | "paths" | "arguments" => {
                        value.is_string()
                            || value
                                .as_sequence()
                                .is_some_and(|s| s.iter().all(yaml_serde::Value::is_string))
                    }
                    _ => true,
                };
                if !valid {
                    return Err(());
                }
            }
        }
        parse_frontmatter(yaml).map_err(|_| ())?
    } else {
        SkillFrontmatter::empty()
    };
    Ok(ImportedSkill {
        source: source.into(),
        frontmatter,
        body: body.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quick_entry_keeps_names_unknown_fields_and_exact_markdown() {
        let source = "---\nname: call-me\nhooks:\n  x: yes\n---\n\n# Do this\n\n";
        let imported = parse(source).unwrap();
        assert_eq!(imported.frontmatter.name.as_deref(), Some("call-me"));
        assert!(imported.frontmatter.extra.contains_key("hooks"));
        assert_eq!(imported.body, "\n# Do this\n\n");
        assert_eq!(imported.source, source);
        assert_eq!(
            parse("# Just instructions\n").unwrap().body,
            "# Just instructions\n"
        );
    }
    #[test]
    fn quick_entry_rejects_unclosed_nonmapping_or_wrongly_typed_frontmatter() {
        for source in [
            "---\nname: x",
            "---\n- item\n---\nbody",
            "---\nname: [wrong]\n---\n",
            "---\nuser-invocable: maybe\n---\n",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
    }
}
