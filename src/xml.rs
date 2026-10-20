// Copyright 2026 yoomin53 (https://github.com/yoomin53)
// SPDX-License-Identifier: Apache-2.0
//
// rcell — 한컴 셀(.cell) 포맷 클린룸 오픈소스 구현

//! 네임스페이스를 해석한 경량 XML DOM.
//!
//! quick-xml 의 이벤트를 트리로 모은다. 요소·속성 이름은 접두사가 아닌
//! 네임스페이스 URI + 로컬 이름으로 저장하므로 한셀(`x:` 접두사)과
//! Excel(기본 네임스페이스) 문서를 같은 코드로 읽을 수 있다.
//!
//! [`resolve_alternate_content`] 는 `mc:AlternateContent` 를 펼친다.
//! 한셀은 `styles.xml` 의 `x:font`, `x:xf` 를 `mc:AlternateContent` 로 감싸서
//! 한컴 확장(`hs:`)을 담기 때문에, 이 처리를 하지 않으면 스타일 인덱스가 어긋난다.

use quick_xml::XmlVersion;
use quick_xml::events::Event;
use quick_xml::name::{QName, ResolveResult};
use quick_xml::reader::NsReader;

use crate::error::{Error, Result};
use crate::ns;

#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub ns: Option<String>,
    /// 원래 접두사 (진단용)
    pub prefix: Option<String>,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    pub ns: Option<String>,
    /// 원래 접두사 (진단용)
    pub prefix: Option<String>,
    pub name: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
    /// 이 요소에서 선언된 네임스페이스 (접두사, URI). 기본 네임스페이스는 접두사 "".
    pub ns_decls: Vec<(String, String)>,
}

fn split_qname(q: &str) -> (Option<&str>, &str) {
    match q.split_once(':') {
        Some((p, l)) => (Some(p), l),
        None => (None, q),
    }
}

fn resolved(r: ResolveResult<'_>) -> Option<String> {
    match r {
        ResolveResult::Bound(n) => Some(n.0.to_string()),
        _ => None,
    }
}

fn xml_err(part: &str, e: impl std::fmt::Display) -> Error {
    Error::Xml {
        part: part.to_string(),
        message: e.to_string(),
    }
}

fn predefined_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "lt" => "<",
        "gt" => ">",
        "amp" => "&",
        "apos" => "'",
        "quot" => "\"",
        _ => return None,
    })
}

fn push_text(stack: &mut [Element], s: &str) {
    let Some(top) = stack.last_mut() else { return };
    if let Some(Node::Text(t)) = top.children.last_mut() {
        t.push_str(s);
    } else {
        top.children.push(Node::Text(s.to_string()));
    }
}

/// XML 문자열을 DOM 으로 파싱한다. `mc:AlternateContent` 는 그대로 둔다.
pub fn parse_raw(xml: &str, part: &str) -> Result<Element> {
    let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
    let mut reader = NsReader::from_str(xml);
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;

    loop {
        let event = reader.read_event().map_err(|e| xml_err(part, e))?;
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let qname = e.name();
                let (prefix, local) = split_qname(qname.0);
                let (res, _) = reader.resolver().resolve_element(qname);
                let mut el = Element {
                    ns: resolved(res),
                    prefix: prefix.map(str::to_string),
                    name: local.to_string(),
                    ..Default::default()
                };
                for a in e.attributes() {
                    let a = a.map_err(|e| xml_err(part, e))?;
                    let key = a.key.0;
                    let value = a
                        .normalized_value(XmlVersion::Implicit1_0)
                        .map_err(|e| xml_err(part, e))?
                        .into_owned();
                    if key == "xmlns" {
                        el.ns_decls.push((String::new(), value));
                        continue;
                    }
                    if let Some(p) = key.strip_prefix("xmlns:") {
                        el.ns_decls.push((p.to_string(), value));
                        continue;
                    }
                    let (aprefix, alocal) = split_qname(key);
                    let (ares, _) = reader.resolver().resolve_attribute(QName(key));
                    el.attrs.push(Attr {
                        ns: resolved(ares),
                        prefix: aprefix.map(str::to_string),
                        name: alocal.to_string(),
                        value,
                    });
                }
                if matches!(event, Event::Start(_)) {
                    stack.push(el);
                } else if let Some(parent) = stack.last_mut() {
                    parent.children.push(Node::Element(el));
                } else {
                    root = Some(el);
                }
            }
            Event::End(_) => {
                let el = stack
                    .pop()
                    .ok_or_else(|| xml_err(part, "짝이 맞지 않는 닫는 태그"))?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(Node::Element(el));
                } else {
                    root = Some(el);
                }
            }
            Event::Text(t) => push_text(&mut stack, &t.xml10_content()),
            Event::CData(t) => push_text(&mut stack, &t.xml10_content()),
            Event::GeneralRef(r) => {
                if let Some(c) = r.resolve_char_ref().map_err(|e| xml_err(part, e))? {
                    push_text(&mut stack, c.encode_utf8(&mut [0; 4]));
                } else if let Some(s) = predefined_entity(&r) {
                    push_text(&mut stack, s);
                } else {
                    return Err(xml_err(part, format!("정의되지 않은 엔티티 &{};", &*r)));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    root.ok_or_else(|| xml_err(part, "루트 요소가 없습니다"))
}

/// XML 문자열을 DOM 으로 파싱하고 `mc:AlternateContent` 를 펼친다.
pub fn parse(xml: &str, part: &str) -> Result<Element> {
    let mut root = parse_raw(xml, part)?;
    resolve_alternate_content(&mut root);
    Ok(root)
}

/// `mc:AlternateContent` 를 선택된 분기의 자식들로 치환한다.
///
/// ECMA-376 Part 3 규칙: `Requires` 의 모든 접두사가 이해되는 네임스페이스인
/// 첫 번째 `mc:Choice` 를 고르고, 없으면 `mc:Fallback` 을 고른다.
pub fn resolve_alternate_content(root: &mut Element) {
    let mut scope: Vec<(String, String)> = Vec::new();
    resolve_in(root, &mut scope);
}

fn lookup<'a>(scope: &'a [(String, String)], prefix: &str) -> Option<&'a str> {
    scope
        .iter()
        .rev()
        .find(|(p, _)| p == prefix)
        .map(|(_, u)| u.as_str())
}

fn resolve_in(el: &mut Element, scope: &mut Vec<(String, String)>) {
    let mark = scope.len();
    scope.extend(el.ns_decls.iter().cloned());

    let old = std::mem::take(&mut el.children);
    for child in old {
        match child {
            Node::Element(c) if c.is(ns::MC, "AlternateContent") => {
                for mut picked in pick_branch(c, scope) {
                    if let Node::Element(e) = &mut picked {
                        resolve_in(e, scope);
                    }
                    el.children.push(picked);
                }
            }
            Node::Element(mut c) => {
                resolve_in(&mut c, scope);
                el.children.push(Node::Element(c));
            }
            text => el.children.push(text),
        }
    }
    scope.truncate(mark);
}

fn pick_branch(ac: Element, scope: &mut Vec<(String, String)>) -> Vec<Node> {
    let mark = scope.len();
    scope.extend(ac.ns_decls.iter().cloned());
    let mut chosen: Option<Element> = None;
    let mut fallback: Option<Element> = None;
    for child in ac.children {
        let Node::Element(branch) = child else {
            continue;
        };
        if branch.is(ns::MC, "Choice") && chosen.is_none() {
            let inner = scope.len();
            scope.extend(branch.ns_decls.iter().cloned());
            let ok = branch
                .attr("Requires")
                .unwrap_or("")
                .split_whitespace()
                .all(|p| lookup(scope, p).is_some_and(|u| ns::UNDERSTOOD.contains(&u)));
            scope.truncate(inner);
            if ok {
                chosen = Some(branch);
            }
        } else if branch.is(ns::MC, "Fallback") {
            fallback = Some(branch);
        }
    }
    scope.truncate(mark);
    chosen.or(fallback).map(|b| b.children).unwrap_or_default()
}

impl Element {
    pub fn is(&self, ns: &str, name: &str) -> bool {
        self.name == name && self.ns.as_deref() == Some(ns)
    }

    /// SpreadsheetML(Transitional/Strict) 요소인지.
    pub fn is_sml(&self, name: &str) -> bool {
        self.name == name && self.ns.as_deref().is_some_and(ns::is_sml)
    }

    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    pub fn sml_children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.elements().filter(move |e| e.is_sml(name))
    }

    pub fn sml_child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.is_sml(name))
    }

    pub fn child_ns(&self, ns: &str, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.is(ns, name))
    }

    /// 네임스페이스가 없는 속성.
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns.is_none() && a.name == name)
            .map(|a| a.value.as_str())
    }

    pub fn attr_ns(&self, ns: &str, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.ns.as_deref() == Some(ns) && a.name == name)
            .map(|a| a.value.as_str())
    }

    /// `r:id` 속성 (Transitional/Strict 관계 네임스페이스).
    pub fn rel_id(&self) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.name == "id" && a.ns.as_deref().is_some_and(ns::is_rel))
            .map(|a| a.value.as_str())
    }

    pub fn attr_u32(&self, name: &str) -> Option<u32> {
        self.attr(name).and_then(|v| v.trim().parse().ok())
    }

    pub fn attr_f64(&self, name: &str) -> Option<f64> {
        self.attr(name).and_then(|v| v.trim().parse().ok())
    }

    /// OOXML 불리언 (`1`/`true`/`on`). 속성이 없으면 `None`.
    pub fn attr_bool(&self, name: &str) -> Option<bool> {
        self.attr(name).map(parse_bool)
    }

    /// 하위 텍스트 전체를 이어 붙인다.
    pub fn text(&self) -> String {
        let mut out = String::new();
        self.collect_text(&mut out);
        out
    }

    fn collect_text(&self, out: &mut String) {
        for c in &self.children {
            match c {
                Node::Text(t) => out.push_str(t),
                Node::Element(e) => e.collect_text(out),
            }
        }
    }

    /// 전위 순회.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Element)) {
        f(self);
        for e in self.elements() {
            e.walk(f);
        }
    }

    /// 이 요소를 XML 로 다시 쓴다 (원래 접두사 그대로). 하위 트리가 쓰는 접두사 선언을 이 요소에
    /// 모아 두므로 다른 문서 안에 그대로 옮겨 넣을 수 있다 (그리기 개체 보존용).
    pub fn to_xml(&self) -> String {
        let mut decls: Vec<(String, String)> = Vec::new();
        let mut add = |p: &Option<String>, u: &Option<String>| {
            if let (Some(p), Some(u)) = (p, u)
                && p != "xml"
                && !decls.iter().any(|(q, _)| q == p)
            {
                decls.push((p.clone(), u.clone()));
            }
        };
        self.walk(&mut |e| {
            add(&e.prefix, &e.ns);
            for a in &e.attrs {
                add(&a.prefix, &a.ns);
            }
        });
        let mut out = String::new();
        self.write_xml(&mut out, &decls, None);
        out
    }

    fn write_xml(
        &self,
        out: &mut String,
        decls: &[(String, String)],
        parent_default: Option<&str>,
    ) {
        let qname = match &self.prefix {
            Some(p) => format!("{p}:{}", self.name),
            None => self.name.clone(),
        };
        out.push('<');
        out.push_str(&qname);
        for (p, u) in decls {
            out.push_str(&format!(" xmlns:{p}=\"{}\"", esc_xml(u)));
        }
        let default = if self.prefix.is_none() {
            self.ns.as_deref()
        } else {
            parent_default
        };
        if self.prefix.is_none() && self.ns.as_deref() != parent_default {
            out.push_str(&format!(
                " xmlns=\"{}\"",
                esc_xml(self.ns.as_deref().unwrap_or(""))
            ));
        }
        for a in &self.attrs {
            match &a.prefix {
                Some(p) => out.push_str(&format!(" {p}:{}=\"{}\"", a.name, esc_xml(&a.value))),
                None => out.push_str(&format!(" {}=\"{}\"", a.name, esc_xml(&a.value))),
            }
        }
        if self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        for c in &self.children {
            match c {
                Node::Text(t) => out.push_str(&esc_xml(t)),
                Node::Element(e) => e.write_xml(out, &[], default),
            }
        }
        out.push_str(&format!("</{qname}>"));
    }
}

fn esc_xml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            _ => o.push(c),
        }
    }
    o
}

/// OOXML `ST_OnOff` / `xsd:boolean` 해석.
pub fn parse_bool(v: &str) -> bool {
    matches!(v.trim(), "1" | "true" | "on" | "True" | "TRUE")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_prefix_and_default_namespace_alike() {
        let a = parse(
            r#"<x:c xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main" r="A1"/>"#,
            "t",
        )
        .unwrap();
        let b = parse(
            r#"<c xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" r="A1"/>"#,
            "t",
        )
        .unwrap();
        assert!(a.is_sml("c") && b.is_sml("c"));
        assert_eq!(a.attr("r"), Some("A1"));
    }

    #[test]
    fn subtree_serializes_with_its_own_namespace_declarations() {
        let doc = parse_raw(
            r#"<xdr:wsDr xmlns:xdr="urn:x" xmlns:a="urn:a" xmlns:r="urn:r"><xdr:sp><a:t xml:space="preserve"> a &amp; b </a:t><a:blip r:embed="rId1"/><n xmlns="urn:d"><m/></n></xdr:sp></xdr:wsDr>"#,
            "t",
        )
        .unwrap();
        let sp = doc.elements().next().unwrap();
        let xml = sp.to_xml();
        assert!(xml.starts_with(r#"<xdr:sp xmlns:xdr="urn:x" xmlns:a="urn:a" xmlns:r="urn:r">"#));
        // 다시 읽으면 같은 트리 (네임스페이스 선언 목록만 다르다)
        let back = parse_raw(&xml, "t").unwrap();
        let strip = |e: &Element| {
            let mut e = e.clone();
            fn clear(e: &mut Element) {
                e.ns_decls.clear();
                for c in &mut e.children {
                    if let Node::Element(x) = c {
                        clear(x);
                    }
                }
            }
            clear(&mut e);
            e
        };
        assert_eq!(strip(&back), strip(sp));
    }

    #[test]
    fn entities_and_char_refs_are_decoded() {
        let e = parse("<t>a &amp; b &#xAC00;&lt;</t>", "t").unwrap();
        assert_eq!(e.text(), "a & b 가<");
    }

    #[test]
    fn alternate_content_prefers_understood_choice() {
        let xml = r#"<x:fonts xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
            <mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
              <mc:Choice xmlns:hs="http://schemas.haansoft.com/office/spreadsheet/8.0" Requires="hs">
                <x:font hs:extension="1"><hs:ratio val="90"/></x:font>
              </mc:Choice>
              <mc:Fallback><x:font/></mc:Fallback>
            </mc:AlternateContent>
            <x:font/>
          </x:fonts>"#;
        let e = parse(xml, "t").unwrap();
        let fonts: Vec<_> = e.sml_children("font").collect();
        assert_eq!(fonts.len(), 2);
        assert_eq!(fonts[0].attr_ns(ns::HS, "extension"), Some("1"));
        assert!(fonts[0].child_ns(ns::HS, "ratio").is_some());
    }

    #[test]
    fn alternate_content_falls_back_for_unknown_namespace() {
        let xml = r#"<root xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
            <mc:AlternateContent>
              <mc:Choice xmlns:zz="urn:unknown" Requires="zz"><a/></mc:Choice>
              <mc:Fallback><b/></mc:Fallback>
            </mc:AlternateContent></root>"#;
        let e = parse(xml, "t").unwrap();
        let names: Vec<_> = e.elements().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["b"]);
    }
}
