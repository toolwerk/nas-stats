//! Reading the `QDocRoot` XML that every QTS CGI answers with.

use nas_model::ProviderError;
use roxmltree::{Document, Node};

/// Parses a CGI answer. Anything that is not XML with a `QDocRoot` root is
/// a [`ProviderError::Parse`], usually a wrong base URL or port.
pub(crate) fn parse_qdoc(body: &str) -> Result<Document<'_>, ProviderError> {
    let doc = Document::parse(body).map_err(|_| not_qts())?;
    if doc.root_element().tag_name().name() != "QDocRoot" {
        return Err(not_qts());
    }
    Ok(doc)
}

/// Text of the direct child `name` of the root, trimmed, CDATA included.
/// Missing or empty is `None`.
pub(crate) fn root_text(doc: &Document<'_>, name: &str) -> Option<String> {
    path_text(doc, &[name])
}

/// Text of the element reached from the root by following, for each name,
/// the first element child of that name. Trimmed, CDATA included; missing
/// or empty is `None`.
pub(crate) fn path_text(doc: &Document<'_>, path: &[&str]) -> Option<String> {
    let (last, parents) = path.split_last()?;
    let mut node = doc.root_element();
    for name in parents {
        node = child(node, name)?;
    }
    child_text(node, last)
}

/// The first element child `name` of `node`.
pub(crate) fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

/// Text of the first element child `name` of `node`, trimmed, CDATA
/// included; missing or empty is `None`.
pub(crate) fn child_text(node: Node<'_, '_>, name: &str) -> Option<String> {
    text(child(node, name)?)
}

/// Text of `node`, trimmed, CDATA included; empty is `None`.
pub(crate) fn text(node: Node<'_, '_>) -> Option<String> {
    let text: String = node
        .children()
        .filter(|n| n.is_text())
        .filter_map(|n| n.text())
        .collect();
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// The number at the start of a value such as `24.8 %` or `1019 RPM`.
/// Whatever follows the number is ignored; no number or a value that is not
/// finite is `None`.
pub(crate) fn number(text: &str) -> Option<f64> {
    let text = text.trim();
    let end = text
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || (i == 0 && (c == '-' || c == '+'))))
        .map_or(text.len(), |(i, _)| i);
    text[..end].parse::<f64>().ok().filter(|v| v.is_finite())
}

fn not_qts() -> ProviderError {
    ProviderError::Parse("keine QTS-Antwort".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_cdata_of_a_root_child() {
        let doc = parse_qdoc(
            "<QDocRoot><authPassed><![CDATA[ 1 ]]></authPassed><empty><![CDATA[]]></empty></QDocRoot>",
        )
        .expect("xml");
        assert_eq!(root_text(&doc, "authPassed").as_deref(), Some("1"));
        assert_eq!(root_text(&doc, "empty"), None);
        assert_eq!(root_text(&doc, "missing"), None);
    }

    #[test]
    fn follows_a_path_of_first_children() {
        let doc = parse_qdoc(
            "<QDocRoot><func><ownContent><root><cpu_tempc>65</cpu_tempc><sys_tempc><![CDATA[ 36 ]]></sys_tempc><empty></empty></root></ownContent></func></QDocRoot>",
        )
        .expect("xml");
        let base = ["func", "ownContent", "root"];
        let at = |name: &str| {
            let mut path = base.to_vec();
            path.push(name);
            path_text(&doc, &path)
        };
        assert_eq!(at("cpu_tempc").as_deref(), Some("65"));
        assert_eq!(at("sys_tempc").as_deref(), Some("36"));
        assert_eq!(at("empty"), None);
        assert_eq!(at("missing"), None);
        assert_eq!(path_text(&doc, &["nothing", "here"]), None);
    }

    #[test]
    fn reads_the_leading_number() {
        assert_eq!(number("24.8 %"), Some(24.8));
        assert_eq!(number(" 1019 RPM"), Some(1019.0));
        assert_eq!(number("15909.0"), Some(15909.0));
        assert_eq!(number("-1"), Some(-1.0));
        assert_eq!(number("n/a"), None);
        assert_eq!(number(""), None);
        assert_eq!(number("%"), None);
    }

    #[test]
    fn other_root_or_html_is_a_parse_error() {
        for body in ["<html><body>Not found</body></html>", "kein xml", ""] {
            assert_eq!(
                parse_qdoc(body).err(),
                Some(ProviderError::Parse("keine QTS-Antwort".into()))
            );
        }
    }
}
