//! Reading the `QDocRoot` XML that every QTS CGI answers with.

use nas_model::ProviderError;
use roxmltree::Document;

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
    let node = doc
        .root_element()
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == name)?;
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
    fn other_root_or_html_is_a_parse_error() {
        for body in ["<html><body>Not found</body></html>", "kein xml", ""] {
            assert_eq!(
                parse_qdoc(body).err(),
                Some(ProviderError::Parse("keine QTS-Antwort".into()))
            );
        }
    }
}
