//------------------------------------------------------------------
// 고정 크기 · 평문 구조 청킹 (SimpleRAG chunk.rs 의 chunk_ids · split_text · split_oversized · split_structured)
//=> 로직은 그대로, cfg() 자리는 ChunkOpts, Tok 자리는 Measure.
//------------------------------------------------------------------

use super::tok::Measure;
use super::ChunkOpts;
use crate::pyutil;
use crate::structure;

//------------------------------------------------------------------
// id 를 창으로 자르기
//=> size 개씩, (size − overlap) 만큼 밀며 자른다. 꼬리가 min 보다 짧으면 버린다.
//
// -in: ids     = 토큰(또는 글자) id
// -in: size    = 창 크기
// -in: overlap = 겹침(size−1 까지만 쓴다)
// -in: min     = 이보다 짧은 조각은 버림
//
// -out: 조각 목록
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn chunk_ids(ids: &[u32], size: usize, overlap: usize, min: usize) -> Vec<Vec<u32>> {
    if ids.is_empty() {
        return vec![];
    }
    // 한 번에 미는 거리 — 겹침이 창보다 크면 1 칸씩
    let step = std::cmp::max(1, size as i64 - std::cmp::min(overlap as i64, size as i64 - 1)) as usize;
    let mut out = vec![];
    let mut start = 0;
    while start < ids.len() {
        let piece = &ids[start..(start + size).min(ids.len())];
        if piece.len() >= min {
            out.push(piece.to_vec());
        }
        if start + size >= ids.len() {
            break;
        }
        start += step;
    }
    out
}

//------------------------------------------------------------------
// 고정 크기 청킹
//
// -in: text    = 글
// -in: m       = 길이 재기
// -in: o       = 옵션(min_tokens)
// -in: size    = 창 크기(None 이면 o.tokens)
// -in: overlap = 겹침(None 이면 o.overlap)
//
// -out: 청크 글 목록(빈 글이면 빈 목록)
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn split_text(text: &str, m: &dyn Measure, o: &ChunkOpts, size: Option<usize>, overlap: Option<usize>) -> Vec<String> {
    if pyutil::strip(text).is_empty() {
        return vec![];
    }
    let size = size.unwrap_or(o.tokens);
    let overlap = overlap.unwrap_or(o.overlap);
    let ids = m.ids(text);
    // id 창을 다시 글로 되돌린 것이 청크다(SimpleRAG 와 같음)
    chunk_ids(&ids, size, overlap, o.min_tokens).iter().map(|p| m.decode(p)).collect()
}

//------------------------------------------------------------------
// 예산을 넘는 본문을 줄 단위로 나누기(한 줄이 넘치면 창으로)
//
// -in: body    = 글
// -in: m       = 길이 재기
// -in: o       = 옵션(min_tokens)
// -in: budget  = 조각 예산
// -in: overlap = 창 겹침
//
// -out: 조각 목록
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn split_oversized(body: &str, m: &dyn Measure, o: &ChunkOpts, budget: usize, overlap: usize) -> Vec<String> {
    let units: Vec<&str> = body.split('\n').filter(|u| !pyutil::strip(u).is_empty()).collect();
    let mut out = vec![];
    let mut cur: Vec<&str> = vec![];
    let mut cur_n = 0;
    for u in units {
        let n = m.tlen(u);
        // 한 줄이 혼자 예산을 넘으면 모아 둔 것을 내보내고 그 줄은 창으로 자른다
        if n > budget {
            if !cur.is_empty() {
                out.push(cur.join("\n"));
                cur.clear();
                cur_n = 0;
            }
            let ids = m.ids(u);
            out.extend(chunk_ids(&ids, budget, overlap, o.min_tokens).iter().map(|p| m.decode(p)));
            continue;
        }
        if cur_n + n > budget {
            out.push(cur.join("\n"));
            cur.clear();
            cur_n = 0;
        }
        cur.push(u);
        cur_n += n;
    }
    if !cur.is_empty() {
        out.push(cur.join("\n"));
    }
    out
}

//------------------------------------------------------------------
// 구조 청킹 (폴백 평문 문서)
//=> 작은 절은 예산 안에서 이어 묶고, 큰 절은 줄 단위로 나눈 뒤 조각마다 접두를 붙인다.
//   표 조각이 머리글 없이 시작하면 그 표의 머리글 줄을 다시 붙인다(상한 이하일 때만).
//    1) structure::split_sections 로 (절 이름표, 본문) 나누기 — 조문 · 번호절 문법을 스스로 찾는다
//    2) 절 하나(접두 포함)가 예산 이하면 모아 두고, 넘치면 모은 것을 내보낸 뒤 줄 단위로 자른다
//    3) 마지막에 min_tokens 미만 청크는 버린다
//
// -in: text    = 평문
// -in: m       = 길이 재기
// -in: o       = 옵션
// -in: title   = 문서 제목(접두에 쓴다)
// -in: size    = 청크 크기(None 이면 o.tokens)
// -in: overlap = 겹침(None 이면 o.overlap)
//
// -out: 청크 글 목록
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn split_structured(text: &str, m: &dyn Measure, o: &ChunkOpts, title: Option<&str>, size: Option<usize>, overlap: Option<usize>) -> Vec<String> {
    if pyutil::strip(text).is_empty() {
        return vec![];
    }
    let size = size.unwrap_or(o.tokens);
    let overlap = overlap.unwrap_or(o.overlap);
    let mut out: Vec<String> = vec![];
    let mut buf: Vec<String> = vec![];
    let mut buf_n = 0;
    // 모아 둔 절들을 한 청크로 내보낸다
    let flush = |buf: &mut Vec<String>, out: &mut Vec<String>| {
        if !buf.is_empty() {
            out.push(pyutil::strip(&buf.join("\n\n")).to_string());
            buf.clear();
        }
    };
    for (section, body) in structure::split_sections(text) {
        // 접두를 끈 옵션이면 붙이지 않는다(SimpleRAG 는 늘 붙임)
        let prefix = if o.prefix && (title.map(|t| !t.is_empty()).unwrap_or(false) || section.is_some()) {
            structure::make_prefix(title, section.as_deref())
        } else {
            String::new()
        };
        let lines: Vec<&str> = body.split('\n').collect();
        let heads = structure::table_headers(&lines);
        let whole = pyutil::strip(&format!("{}{}", prefix, body)).to_string();
        let n = m.tlen(&whole);
        if n <= size {
            if buf_n + n > size {
                flush(&mut buf, &mut out);
                buf_n = 0;
            }
            buf.push(whole);
            buf_n += n;
            continue;
        }
        flush(&mut buf, &mut out);
        buf_n = 0;
        // 접두 길이만큼 예산을 줄인다(최소 min_tokens)
        let budget = std::cmp::max(o.min_tokens, size.saturating_sub(m.tlen(&prefix)));
        for piece in split_oversized(&body, m, o, budget, overlap) {
            if pyutil::strip(&piece).is_empty() {
                continue;
            }
            let mut piece = piece;
            let first = pyutil::lstrip(&piece).split('\n').next().unwrap_or("").to_string();
            // 표 행으로 시작하는 조각이면 그 표의 머리글 줄을 앞에 다시 붙인다
            if !heads.is_empty() && first.starts_with('|') {
                let hdr = heads.iter().find(|(i, _)| pyutil::strip(lines[**i]) == pyutil::strip(&first)).map(|(_, h)| h.clone());
                if let Some(h) = hdr {
                    if !piece.contains(&h) && m.tlen(&h) <= o.table_header_max_tokens {
                        piece = format!("{}\n{}", h, piece);
                    }
                }
            }
            out.push(pyutil::strip(&format!("{}{}", prefix, piece)).to_string());
        }
    }
    flush(&mut buf, &mut out);
    out.into_iter().filter(|c2| m.tlen(c2) >= o.min_tokens).collect()
}
