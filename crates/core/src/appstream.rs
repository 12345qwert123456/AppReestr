//! AppStream metainfo files (`/usr/share/metainfo/*.xml`).

use crate::model::{ComponentKind, MetaInfo};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

fn kind_from_attr(t: &str) -> ComponentKind {
    match t {
        "desktop-application" | "desktop" => ComponentKind::DesktopApplication,
        "console-application" => ComponentKind::ConsoleApplication,
        "web-application" => ComponentKind::WebApplication,
        "runtime" => ComponentKind::Runtime,
        "font" => ComponentKind::Font,
        "codec" => ComponentKind::Codec,
        "addon" => ComponentKind::Addon,
        "driver" => ComponentKind::Driver,
        "firmware" => ComponentKind::Firmware,
        "localization" => ComponentKind::Localization,
        "inputmethod" => ComponentKind::InputMethod,
        "icon-theme" => ComponentKind::IconTheme,
        _ => ComponentKind::Other,
    }
}

fn resolve_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => return None,
    })
}

/// Parse a metainfo document. `locale` like `ru_RU` selects translated name and summary.
pub fn parse(xml: &str, locale: Option<&str>) -> Option<MetaInfo> {
    // No trim_text: it would eat the spaces around entity references, which arrive as
    // separate events. Values are trimmed when an element ends.
    let mut reader = Reader::from_str(xml);

    let lang = locale.and_then(|l| l.split('_').next());
    let mut stack: Vec<String> = Vec::new();
    let mut kind = None;
    let mut id = None;
    let (mut name, mut name_l10n, mut summary, mut summary_l10n) = (None, None, None, None);
    let mut developer = None;
    let mut launchables = Vec::new();

    // State of the element whose text we are collecting.
    let mut text = String::new();
    let mut cur_lang: Option<String> = None;
    let mut cur_launchable_desktop = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let tag = e.local_name().as_ref().to_string();
                if stack.is_empty() && (tag == "component" || tag == "application") {
                    let t = e
                        .try_get_attribute("type")
                        .ok()
                        .flatten()
                        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()))
                        .unwrap_or_else(|| "generic".into());
                    kind =
                        Some(if tag == "application" { ComponentKind::DesktopApplication } else { kind_from_attr(&t) });
                }
                cur_lang = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref() == "xml:lang")
                    .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok().map(|v| v.into_owned()));
                cur_launchable_desktop = tag == "launchable"
                    && e.try_get_attribute("type").ok().flatten().is_some_and(|a| a.value.as_ref() == "desktop-id");
                text.clear();
                stack.push(tag);
            }
            Ok(Event::Text(t)) => {
                text.push_str(&t.xml_content(XmlVersion::Implicit1_0));
            }
            Ok(Event::GeneralRef(r)) => {
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    text.push(c);
                } else if let Some(c) = resolve_entity(&r) {
                    text.push(c);
                }
            }
            Ok(Event::End(_)) => {
                let tag = stack.pop().unwrap_or_default();
                let parent = stack.last().map(String::as_str);
                let depth = stack.len();
                let value = text.trim().to_string();
                let lang_ok = |l: &Option<String>| match (l.as_deref(), lang, locale) {
                    (Some(l), Some(lang), Some(loc)) => l == loc || l == lang,
                    _ => false,
                };
                if depth == 1 && !value.is_empty() {
                    match tag.as_str() {
                        "id" => id = Some(value.clone()),
                        "name" if cur_lang.is_none() => name = Some(value.clone()),
                        "name" if lang_ok(&cur_lang) => name_l10n = Some(value.clone()),
                        "summary" if cur_lang.is_none() => summary = Some(value.clone()),
                        "summary" if lang_ok(&cur_lang) => summary_l10n = Some(value.clone()),
                        "developer_name" if cur_lang.is_none() => developer = Some(value.clone()),
                        "launchable" if cur_launchable_desktop => launchables.push(value.clone()),
                        _ => {}
                    }
                } else if depth == 2 && tag == "name" && parent == Some("developer") && cur_lang.is_none() {
                    developer = Some(value.clone());
                }
                text.clear();
                cur_lang = None;
                cur_launchable_desktop = false;
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }

    Some(MetaInfo {
        id: id?,
        kind: kind?,
        name: name_l10n.or(name),
        summary: summary_l10n.or(summary),
        launchables,
        developer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_component() {
        let xml = r#"<?xml version="1.0"?>
<component type="desktop-application">
  <id>org.gnome.Calculator</id>
  <name>Calculator</name>
  <name xml:lang="ru">Калькулятор</name>
  <summary>Perform arithmetic &amp; more</summary>
  <developer id="org.gnome"><name>The GNOME Project</name></developer>
  <launchable type="desktop-id">org.gnome.Calculator.desktop</launchable>
  <releases><release version="1"><description><p>x</p></description></release></releases>
</component>"#;
        let m = parse(xml, Some("ru_RU")).unwrap();
        assert_eq!(m.id, "org.gnome.Calculator");
        assert_eq!(m.kind, ComponentKind::DesktopApplication);
        assert_eq!(m.name.as_deref(), Some("Калькулятор"));
        assert_eq!(m.summary.as_deref(), Some("Perform arithmetic & more"));
        assert_eq!(m.developer.as_deref(), Some("The GNOME Project"));
        assert_eq!(m.launchables, vec!["org.gnome.Calculator.desktop"]);
    }

    #[test]
    fn runtime_kind() {
        let m = parse(r#"<component type="font"><id>noto</id></component>"#, None).unwrap();
        assert_eq!(m.kind, ComponentKind::Font);
    }
}
