//------------------------------------------------------------------
// 청크의 실제 쪽 찾기 (SimpleRAG chunk.rs 의 PageFinder · unit_pages)
//=> 청킹은 절의 단위(제목·블록·표 행)를 이어 붙여 자르므로, 청크는 절 글의 한 토막이다.
//   그래서 절 글(공백 뺀)과 글자마다의 쪽을 만들어 두고, 청크 머리·꼬리가 절 글 어디에 있는지 찾는다.
//    1) 청크 앞의 접두 "[문서 > 절]\n" 은 뗀다(절 글에는 없다)
//    2) 머리 24자를 첫 절 글에서 찾는다 — 같은 절의 앞 청크 자리 뒤부터(같은 글귀가 반복돼도 순서대로).
//       표 머리글을 다시 붙인 조각은 머리가 앞쪽 표 머리에 걸리므로 40·80·120자 뒤에서도 찾아 본다
//    3) 꼬리 24자를 마지막 절 글에서 찾는다(머리 자리 뒤부터) → 끝 쪽
//   Python chunk.py _PageFinder 와 같은 규칙.
//------------------------------------------------------------------

use super::assemble::Section;
use crate::pyutil;

const FIND_LEN: usize = 24;                 // 청크 머리·꼬리에서 대조할 글자 수(공백 뺀)
const FIND_TRY: [usize; 4] = [0, 40, 80, 120]; // 머리 대조가 안 되면 이만큼 뒤에서 다시(표 머리글을 다시 붙인 조각)
const FIND_MIN: usize = 8;                  // 이보다 짧으면 흔한 글이라 대조하지 않는다

//------------------------------------------------------------------
// 쪽 찾기 상태
// -필드: secs   = 절 번호 → (절 글, 바이트마다의 쪽, 다음 찾기 시작 자리)
// -필드: prefix = 청크에 접두가 붙어 있나(붙어 있을 때만 떼고 대조한다)
//------------------------------------------------------------------
pub struct PageFinder {
    secs: std::collections::HashMap<i64, (String, Vec<Option<i64>>, usize)>,
    prefix: bool,
}

impl PageFinder {
    //------------------------------------------------------------------
    // 새 쪽 찾기
    //
    // -in: prefix = 청크에 "[문서 > 절]\n" 접두가 붙어 있나
    //
    // -out: PageFinder
    // -out: error = 예외 없음
    //------------------------------------------------------------------
    pub fn new(prefix: bool) -> PageFinder {
        PageFinder { secs: Default::default(), prefix }
    }

    // 절 글 준비(한 번 만들어 둔다)
    fn sec(&mut self, sec: &Section) -> &mut (String, Vec<Option<i64>>, usize) {
        self.secs.entry(sec.id).or_insert_with(|| {
            let mut hay = String::new();
            let mut pg = vec![];
            for (t, p) in unit_pages(sec) {
                for ch in t.chars().filter(|c| !pyutil::is_space(*c)) {
                    hay.push(ch);
                    // 바이트 자리로 찾으므로 글자의 바이트 수만큼 같은 쪽을 채운다
                    pg.extend(std::iter::repeat(p).take(ch.len_utf8()));
                }
            }
            (hay, pg, 0)
        })
    }

    //------------------------------------------------------------------
    // 청크 → 쪽
    //
    // -in: text = 청크 글
    // -in: secs = 청크가 걸친 절(첫 절 · 마지막 절을 쓴다)
    //
    // -out: Some((시작 쪽, 최소 쪽, 최대 쪽))
    // -out: error = 대조 실패면 None
    //------------------------------------------------------------------
    pub fn locate(&mut self, text: &str, secs: &[Section]) -> Option<(i64, i64, i64)> {
        // 접두는 "[문서 > 절]\n" 모양이다(make_prefix) — 접두를 끈 청크는 그대로 대조
        let body = match (self.prefix && text.starts_with('['), text.find("]\n")) {
            (true, Some(i)) => &text[i + 2..],
            _ => text,
        };
        let sq: Vec<char> = body.chars().filter(|c| !pyutil::is_space(*c)).collect();
        let (start_pos, p0) = {
            let (hay, pg, cur) = self.sec(&secs[0]);
            let mut hit: Option<usize> = None;
            for off in FIND_TRY {
                let needle: String = sq.iter().skip(off).take(FIND_LEN).collect();
                if needle.chars().count() < FIND_MIN {
                    break;
                }
                // 앞 청크 자리 뒤에서 먼저, 없으면 절 처음부터
                hit = hay[*cur..].find(&needle).map(|i| i + *cur).or_else(|| hay.find(&needle));
                if hit.is_some() {
                    break;
                }
            }
            let pos = hit?;
            let p0 = pg[pos]?;
            // 다음 청크는 이 자리 뒤부터(겹침이 있어 끝이 아니라 시작 뒤로 둔다)
            *cur = pos + hay[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            (pos, p0)
        };
        // 꼬리 — 마지막 절 글에서
        let tail: String = sq[sq.len().saturating_sub(FIND_LEN)..].iter().collect();
        let same = secs.len() == 1;
        let (hay, pg, _) = self.sec(&secs[secs.len() - 1]);
        let from = if same { start_pos } else { 0 };
        let p1 = if tail.chars().count() >= FIND_MIN {
            hay[from..].find(&tail).map(|i| i + from + tail.len() - 1).and_then(|e| pg.get(e).copied().flatten())
        } else {
            None
        };
        let p1 = p1.unwrap_or(p0);
        Some((p0, p0.min(p1), p0.max(p1)))
    }
}

//------------------------------------------------------------------
// 절 단위와 그 쪽 (section_units 와 같은 순서·같은 글)
//=> 제목은 제목 블록의 쪽, 표 머리글·행은 표 블록의 쪽, 그 밖은 블록의 쪽.
//
// -in: sec = 절
//
// -out: (글, 쪽) 목록
// -out: error = 예외 없음
//------------------------------------------------------------------
fn unit_pages(sec: &Section) -> Vec<(String, Option<i64>)> {
    let mut out = vec![];
    if let Some(h) = &sec.heading {
        if !h.synthetic && !pyutil::strip(&h.text).is_empty() {
            out.push((h.text.clone(), h.page));
        }
    }
    for b in &sec.blocks {
        // 표든 문단이든 공백을 빼고 대조하므로 블록 글 전체를 그 블록의 쪽으로 둔다
        if !pyutil::strip(&b.text).is_empty() {
            out.push((b.text.clone(), b.page));
        }
    }
    out
}
