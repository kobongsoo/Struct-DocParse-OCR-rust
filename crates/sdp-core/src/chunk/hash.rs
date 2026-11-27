//------------------------------------------------------------------
// 청크 해시 (0.3.1~) — 중복 청크를 알아보는 지문 두 가지
//=> hash      = 청크 글(text) 그대로의 지문. 임베딩에 들어가는 글이 같으면 같다 → 임베딩 재사용 · 같은 사본 찾기
//   body_hash = 접두 줄("[문서 > 절]")을 떼고 공백을 정리한 본문의 지문. 다른 문서에 든 같은 본문도 같다 → 중복 합치기
//   둘 다 SHA-256(UTF-8) 의 앞 16자리(소문자 16진수, 64비트). 어느 언어에서나 같은 값을 다시 만들 수 있다.
//
//   본문 규칙(body_of):
//    1) 줄 단위로 보고, 접두 줄을 뺀다. 접두 줄 = 앞뒤 공백을 떼면 "[제목]" 또는 "[제목 > …]" 인 줄
//       (제목 = 문서 제목, 공백은 한 칸으로). 작은 절을 묶은 청크는 접두 줄이 여러 개라 모두 뺀다.
//       제목이 비었으면 첫 줄이 "[…]" 일 때만 접두로 본다 — 본문의 "[별표 1]" 같은 줄을 지우지 않으려고.
//    2) 남은 줄을 공백 한 칸으로 잇고, 연속 공백(줄바꿈 · 탭 포함)을 한 칸으로 줄이고 앞뒤를 자른다.
//    3) 그렇게 해서 빈 글이 되면(접두 줄만 든 청크) 접두를 떼지 않은 청크 글을 2) 처럼 정리해 쓴다.
//------------------------------------------------------------------

use sha2::{Digest, Sha256};

use crate::pyutil;

// 해시 글자 수 — SHA-256 앞 16자리(64비트). 청크 수백만 개에서도 우연히 겹칠 확률은 사실상 0
const HEX_LEN: usize = 16;

//------------------------------------------------------------------
// 글 → SHA-256 앞 16자리
//
// -in: s = 글(UTF-8 바이트로 해시)
//
// -out: 16자리 소문자 16진수
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn sha16(s: &str) -> String {
    let d = Sha256::digest(s.as_bytes());
    let mut out = String::with_capacity(HEX_LEN);
    for b in d.iter().take(HEX_LEN / 2) {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

//------------------------------------------------------------------
// 연속 공백을 한 칸으로 줄이고 앞뒤를 자른다 (Python " ".join(s.split()) 과 같다)
//
// -in: s = 글
//
// -out: 정리된 글
// -out: error = 예외 없음
//------------------------------------------------------------------
fn squash(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for w in s.split(pyutil::is_space).filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

//------------------------------------------------------------------
// 접두 줄인가
//
// -in: line  = 줄 하나(앞뒤 공백 포함 가능)
// -in: title = 정리된 문서 제목(빈 글이면 제목 없음)
//
// -out: true = 접두 줄
// -out: error = 예외 없음
//------------------------------------------------------------------
fn is_prefix_line(line: &str, title: &str) -> bool {
    let l = squash(line);
    if !(l.starts_with('[') && l.ends_with(']')) {
        return false;
    }
    if title.is_empty() {
        return true;
    }
    // 접두는 언제나 문서 제목으로 시작한다(길이를 줄일 때도 제목은 남는다)
    l == format!("[{}]", title) || l.starts_with(&format!("[{} > ", title))
}

//------------------------------------------------------------------
// 청크 글 → 본문(접두 줄 떼고 공백 정리)
//
// -in: text  = 청크 글
// -in: title = 문서 제목
//
// -out: 본문 글(body_hash 를 만드는 글)
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn body_of(text: &str, title: &str) -> String {
    let t = squash(title);
    let mut kept: Vec<&str> = vec![];
    for (i, line) in text.lines().enumerate() {
        // 제목이 없으면 맨 첫 줄만 접두 후보로 본다
        let candidate = !t.is_empty() || i == 0;
        if candidate && is_prefix_line(line, &t) {
            continue;
        }
        kept.push(line);
    }
    let body = squash(&kept.join(" "));
    // 접두 줄만 든 청크(목차처럼 제목만 있는 절)는 접두가 곧 내용이다 — 떼면 빈 글이 되어
    // 서로 다른 청크가 모두 같은 지문이 되므로, 이때는 접두를 떼지 않은 글을 쓴다
    if body.is_empty() {
        return squash(text);
    }
    body
}

//------------------------------------------------------------------
// 청크 하나의 (hash, body_hash)
//
// -in: text  = 청크 글
// -in: title = 문서 제목
//
// -out: (hash, body_hash)
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn hashes(text: &str, title: &str) -> (String, String) {
    (sha16(text), sha16(&body_of(text, title)))
}
