//------------------------------------------------------------------
// OOXML 공통 도우미 (Python docmodel/ooxml_common.py)
//=> ZIP 안 XML 읽기, 관계(.rels) 풀기, 엑셀 칸 주소 풀기.
//------------------------------------------------------------------

use std::io::Read;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::xmldom::{self, Node};

pub type Zip = zip::ZipArchive<std::fs::File>;

//------------------------------------------------------------------
// ZIP 열기
//------------------------------------------------------------------
pub fn open(path: &str, what: &str) -> Result<Zip, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("{} 열기 실패: {}", what, e))?;
    zip::ZipArchive::new(f).map_err(|_| format!("{} 열기 실패: BadZipFile", what))
}

// 항목 이름 목록
pub fn names(z: &Zip) -> Vec<String> {
    z.file_names().map(|s| s.to_string()).collect()
}

//------------------------------------------------------------------
// ZIP 항목 하나를 XML 트리로 (없거나 깨졌으면 None)
//------------------------------------------------------------------
pub fn parse(z: &mut Zip, name: &str) -> Option<Node> {
    let mut e = z.by_name(name).ok()?;
    let mut b = vec![];
    e.read_to_end(&mut b).ok()?;
    xmldom::parse(&b).ok()
}

// 속성(지역 이름)
pub fn attr<'a>(el: Option<&'a Node>, name: &str) -> Option<&'a str> {
    el.and_then(|e| e.get(name))
}

// 자식(지역 이름)
pub fn child<'a>(el: Option<&'a Node>, name: &str) -> Option<&'a Node> {
    el.and_then(|e| e.child(name))
}

// posixpath.normpath — "a/b/../c" → "a/c"
pub fn normpath(p: &str) -> String {
    let mut out: Vec<&str> = vec![];
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if out.last().map(|l| *l != "..").unwrap_or(false) {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            x => out.push(x),
        }
    }
    let s = out.join("/");
    if s.is_empty() { ".".into() } else { s }
}

//------------------------------------------------------------------
// 부품의 관계 표 — {Id: (종류 끝 이름, 대상 전체 경로)}
//=> 외부 링크(TargetMode=External)는 뺀다. 문서 순서를 지킨다.
//------------------------------------------------------------------
pub fn rels(z: &mut Zip, part: &str) -> Vec<(String, (String, String))> {
    let (folder, base) = match part.rfind('/') {
        Some(i) => (&part[..i], &part[i + 1..]),
        None => ("", part),
    };
    let rel_path = if folder.is_empty() { format!("_rels/{}.rels", base) } else { format!("{}/_rels/{}.rels", folder, base) };
    let mut out: Vec<(String, (String, String))> = vec![];
    let root = match parse(z, &rel_path) {
        Some(r) => r,
        None => return out,
    };
    for r in &root.children {
        if r.tag != "Relationship" {
            continue;
        }
        let target = r.get("Target").unwrap_or("");
        if r.get("TargetMode") == Some("External") {
            continue;
        }
        let full = if target.starts_with('/') {
            target.trim_start_matches('/').to_string()
        } else if folder.is_empty() {
            normpath(target)
        } else {
            normpath(&format!("{}/{}", folder, target))
        };
        let kind = r.get("Type").unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
        let id = r.get("Id").unwrap_or("").to_string();
        // 같은 Id 가 또 나오면 자리는 그대로 두고 값만 바꾼다(파이썬 dict 와 같다)
        match out.iter_mut().find(|(k, _)| *k == id) {
            Some(slot) => slot.1 = (kind, full),
            None => out.push((id, (kind, full))),
        }
    }
    out
}

static CELL_REF: Lazy<Regex> = Lazy::new(|| Regex::new(r"^([A-Z]+)(\d+)$").unwrap());

//------------------------------------------------------------------
// "B3" → (2, 1)  (0부터 행, 열)
//------------------------------------------------------------------
pub fn cell_ref(r: &str) -> Option<(i64, i64)> {
    let up = r.to_uppercase();
    let m = CELL_REF.captures(&up)?;
    let mut col = 0i64;
    for ch in m.get(1).unwrap().as_str().chars() {
        col = col * 26 + (ch as i64 - 64);
    }
    let row: i64 = m.get(2).unwrap().as_str().parse().ok()?;
    Some((row - 1, col - 1))
}

// 파이썬 str.isdigit() 근사(아스키 숫자만, 비어 있지 않음)
pub fn isdigit(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

// 이름 끝 숫자
pub fn num_key(name: &str, re: &Regex) -> i64 {
    re.captures(name).and_then(|c| c.get(1)).and_then(|m| m.as_str().parse().ok()).unwrap_or(0)
}

//------------------------------------------------------------------
// SmartArt 데이터 부품(dataN.xml)의 글 (본문밖글읽기 설계서 7-2)
//=> dgm:ptLst 의 점(dgm:pt) 가운데 글이 있는 점만 읽는다 — type 이 없거나 node · asst.
//   parTrans · sibTrans(연결선) · pres(모양) · doc(뿌리)는 건너뛴다. 점 하나가 글 하나이고,
//   점 안 문단(a:p)이 여럿이면 줄바꿈으로 잇는다. drawingN.xml(그려 둔 모양 캐시)은 같은 글이
//   두 번 들어가므로 읽지 않는다.
//
// -in: z    = 열린 ZIP
// -in: part = 데이터 부품 전체 경로(예: "ppt/diagrams/data1.xml")
//
// -out: 글 목록(문서 순서). 부품이 없거나 깨졌으면 빈 목록
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn diagram_texts(z: &mut Zip, part: &str) -> Vec<String> {
    let root = match parse(z, part) {
        Some(r) => r,
        None => return vec![],
    };
    let mut out = vec![];
    for pt in root.iter_tag("pt") {
        // 글을 담는 점만 — 연결선 · 모양 점에는 글이 없거나 되풀이된 글이다
        if !matches!(pt.get("type"), None | Some("node") | Some("asst")) {
            continue;
        }
        // dgm:t(점의 글 상자) — 안의 a:p 마다 a:t 글을 모은다
        let body = match pt.child("t") {
            Some(t) => t,
            None => continue,
        };
        let mut lines = vec![];
        for p in body.iter_tag("p") {
            let mut buf = String::new();
            for e in p.iter() {
                if e.tag == "t" {
                    buf.push_str(&e.text);
                } else if e.tag == "br" {
                    buf.push('\n');
                }
            }
            if !crate::pyutil::strip(&buf).is_empty() {
                lines.push(buf);
            }
        }
        let s = crate::pyutil::strip(&lines.join("\n")).to_string();
        if !s.is_empty() {
            out.push(s);
        }
    }
    out
}

//------------------------------------------------------------------
// 부품의 SmartArt 표 — {관계 Id: 글 목록}
//=> 부품 관계 가운데 diagramData 를 따라가 글을 읽는다. 본문이 r:dm="Id" 로 가리키는 자리에 넣을 때 쓴다.
//
// -in: z    = 열린 ZIP
// -in: part = 슬라이드 · 본문 부품 경로
//
// -out: (Id, 글 목록) 목록 — 관계 순서. 글 없는 데이터 부품은 뺀다
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn diagrams_of(z: &mut Zip, part: &str) -> Vec<(String, Vec<String>)> {
    let mut out = vec![];
    for (id, (kind, target)) in rels(z, part) {
        if kind == "diagramData" {
            let texts = diagram_texts(z, &target);
            if !texts.is_empty() {
                out.push((id, texts));
            }
        }
    }
    out
}

//------------------------------------------------------------------
// 요소 안의 SmartArt 가리킴(dgm:relIds 의 r:dm) Id 들 — 문서 순서
//------------------------------------------------------------------
pub fn diagram_refs(el: &Node) -> Vec<String> {
    el.iter().into_iter().filter(|e| e.tag == "relIds").filter_map(|e| e.get("dm").map(|s| s.to_string())).collect()
}

//------------------------------------------------------------------
// 시트 관계로 찾은 셀 메모 부품 (본문밖글읽기 설계서 7-3)
//=> 메모 부품은 지금까지 파일 이름(xl/comments1.xml 등)으로 찾았다. openpyxl 같은 도구는
//   xl/comments/comment1.xml 처럼 다른 이름으로 저장하므로, 시트의 관계(comments · threadedComment)를
//   따라가 이름으로 못 찾은 부품만 더한다. 이름으로 찾던 것은 순서까지 그대로 둔다(결과가 같게).
//
// -in: z      = 열린 ZIP
// -in: sheets = 시트 부품 경로(시트 순서)
// -in: have   = 이미 고른 (부품, 스레드 댓글인가) 목록
//
// -out: 더할 (부품, 스레드 댓글인가) 목록 — 시트 순서
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn more_comment_parts(z: &mut Zip, sheets: &[String], have: &[(String, bool)]) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = vec![];
    for sh in sheets {
        for (_, (kind, target)) in rels(z, sh) {
            let threaded = match kind.as_str() {
                "comments" => false,
                "threadedComment" => true,
                _ => continue,
            };
            // 이름으로 이미 찾았거나 앞 시트에서 더한 부품은 다시 넣지 않는다
            if have.iter().any(|(n, _)| *n == target) || out.iter().any(|(n, _)| *n == target) {
                continue;
            }
            out.push((target, threaded));
        }
    }
    out
}
