//------------------------------------------------------------------
// OCR 줄 → 구조 블록 (OCR 구조분석 1단계 설계서 2026-11-16)
//=> 캐시에 있는 OCR 상자 좌표만으로 다단 · 문단 · 제목 · 목록 · 머리말/꼬리말/쪽번호를 가린다.
//   모델을 더 쓰지 않고, 파일 · 엔진 · 설정을 건드리지 않는 순수 함수다(단위 시험이 쉽게).
//    1) 띠 · 단 나누기 — 세로 빈 띠(gutter)로 단을 찾고, 단을 가로지르는 상자(제목 등)로 위아래를 나눈다
//    2) 줄 묶기 — 단마다 기존 group_rows 규칙 그대로
//    3) 쪽 통계 — 본문 글자 높이 h
//    4) 라벨 — 가장자리(머리말·꼬리말·쪽번호) → 표 행 → 제목(높이 · 번호) → 목록
//    5) 문단 묶기 — 줄 간격 · 왼쪽 정렬 · 오른쪽 끝까지 찬 줄
//   단이 하나뿐인 쪽은 1)이 아무것도 나누지 않아 2)의 줄이 이전(group_rows 한 번)과 같다.
//   숫자 문턱은 모두 시작값이다 — 측정(tools/ocr_compare/eval_layout.py)으로 맞춘다.
//------------------------------------------------------------------

use std::collections::HashMap;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::docmodel::blocks::{self as B, Block};
use crate::pyutil;
use crate::structure;

// ── 띠 · 단 ──
// 단 사이 빈 띠의 최소 너비(× 상자 높이 중앙값)
const GUTTER_MIN: f32 = 1.0;
// 빈 띠 가운데가 글 범위의 이 비율 안에 있어야 한다(오른쪽 끝 쪽번호 · 여백 글자로 자르지 않게)
const GUTTER_POS: (f32, f32) = (0.2, 0.8);
// 빈 띠를 지나가도 되는 상자 수(제목처럼 단을 가로지르는 상자) — 전체의 이 비율까지
const GUTTER_CROSS: f32 = 0.15;
// 단 한쪽에 있어야 하는 최소 상자 수
const MIN_COL_BOXES: usize = 3;
// 두 단의 세로 범위가 이만큼 겹쳐야 나란한 단이다(짧은 쪽 기준)
const COL_OVERLAP: f32 = 0.5;
// 단 안 상자 너비 중앙값 ≥ 단 너비 × 이 값 — 글줄이 단을 채워야 한다(표의 짧은 칸과 가르기)
const COL_FILL: f32 = 0.45;
// 단 안 상자 글자 수 중앙값 하한 — 표 칸("구분", "12") 같은 짧은 글은 단으로 보지 않는다
const COL_MIN_CHARS: usize = 5;
// 단 안에서 다시 나누는 깊이(2 = 3단까지)
const MAX_SPLIT_DEPTH: usize = 2;

// ── 쪽 통계 ──
// 본문 높이를 잴 때 쓰는 줄의 최소 글자(숫자 · 기호 뺀) 수
const BODY_MIN_CHARS: usize = 10;
// 쪽 값으로 본문 높이를 쓰려면 이만큼의 본문 줄(글자 BODY_MIN_CHARS 이상)이 필요하다
const BODY_MIN_ROWS: usize = 8;

// ── 라벨 ──
// 머리말 · 꼬리말 위치 — 쪽 높이의 위 · 아래 이 비율 안
const EDGE_ZONE: f32 = 0.12;
// 흔들림 허용 머리말: 쪽 위 · 아래 몇 줄 · 유사도 · 최소 쪽 수
const EDGE_ROWS: usize = 2;
const EDGE_SIM: f32 = 0.75;
const MIN_EDGE_PAGES: usize = 3;
// 표 행: 상자가 이만큼 이상이고, 넓은 틈(× h)이 이만큼 이상
//   (빈 칸이 있는 2칸 행 "이름   시뮬레이션 이름" 도 잡도록 상자 2개 · 넓은 틈 1개부터)
const TABLE_MIN_BOXES: usize = 2;
const TABLE_GAP: f32 = 2.0;
const TABLE_MIN_GAPS: usize = 1;
// 높이 제목: 줄 높이 ≥ h × 이 값
const HEAD_RATIO: f32 = 1.2;
const HEAD_MAX_CHARS: usize = 60;
// 높이 제목 문서 단위 안전장치 — 후보 수 하한 · 전체 줄 중 비율 상한
const MIN_SIZE_HEADS: usize = 3;
// 이만큼 크면(× h) 문서에 하나뿐이어도 제목으로 본다(표지 · 장 첫 쪽의 큰 제목)
const HEAD_SURE_RATIO: f32 = 1.5;
// 이만큼 크면(× h) '뒤에 본문' · '덩어리 끝까지 찬 줄' 검사를 건너뛴다 — 혼자 한 덩어리인 큰 쪽 제목(mc04 "민감한 데이터 노출" 2.7배)
const HEAD_HUGE_RATIO: f32 = 2.0;
// 높이 제목 뒤 이만큼(× h) 안, 줄 몇 개 안에 본문 줄이 있어야 한다(화면 캡처 속 큰 버튼 글 거르기)
const HEAD_BODY_WITHIN: f32 = 4.0;
const HEAD_BODY_LOOKAHEAD: usize = 3;
const HEAD_MAX_SHARE: f32 = 0.15;
// 높이 비율이 이만큼 이상 차이 나면 다른 수준 — 같은 글꼴 제목도 OCR 상자 높이가 ±0.1 흔들린다
const LEVEL_STEP: f32 = 0.25;
const MAX_SIZE_LEVEL: i64 = 3;
// 번호 제목: 글자 수 상한 · 바로 위 줄과의 틈(× h) 하한
const NUM_HEAD_MAX_CHARS: usize = 40;
const NUM_HEAD_GAP: f32 = 0.8;
// 번호 제목은 번호라는 근거가 있어 제목이 빽빽한 문서도 허용한다(줄의 30%까지)
const NUM_HEAD_MAX_SHARE: f32 = 0.3;
const MAX_LEVEL: i64 = 6;
// 번호 제목 켬 — 표 행 오판이 많으면 이것만 끈다(설계서 §12 ④)
const NUM_HEADINGS: bool = true;

// ── 문단 묶기 ──
// 문단 안 줄 틈은 h 의 0.3~0.4배, 문단 사이는 0.8배 안팎(scan03 측정) — 그 사이
const PARA_GAP: f32 = 0.6;
const PARA_H_RATIO: (f32, f32) = (0.8, 1.25);
const PARA_LEFT_TOL: f32 = 1.0;
const PARA_RIGHT_TOL: f32 = 3.0;
const PARA_MAX_ROWS: usize = 40;

// 줄 머리 목록 기호 — 기호 · 동그라미 숫자 · 1) · (1) · 가) · (가) · 가.
static LIST_HEAD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(
        r"^(?:[•·○●■□▪▶◦※◆◇►▷▸◎∙]\s*",
        r"|[-–]\s+",
        r"|[①-⑳]\s*",
        r"|\(?\d{1,2}\)\s*",
        r"|\(?[가나다라마바사아자차카타파하]\)\s*",
        r"|[가나다라마바사아자차카타파하]\.\s+)\S"
    )).unwrap()
});
// 번호 제목 꼴 — 키는 수준을 정하는 '모양'
static NUM_FORMS: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    vec![
        ("편장", Regex::new(r"^제\s*\d+\s*[편장]\s*\S").unwrap()),
        ("절", Regex::new(r"^제\s*\d+\s*절\s*\S").unwrap()),
        ("로마", Regex::new(r"^[ⅠⅡⅢⅣⅤⅥⅦⅧⅨⅩ]+\s*[.．]?\s*\S").unwrap()),
        ("가나다", Regex::new(r"^[가나다라마바사아자차카타파하]\.\s*\S").unwrap()),
        // 부록 번호 "A.1" · "B.5.2"
        ("영문번호", Regex::new(r"^[A-Z]\.\d{1,2}(?:\.\d{1,2})*\.?\s*[^\d\s.]").unwrap()),
    ]
});
// 숫자 번호 "1." · "1.2" · "1.2.3.4" — 마디 수가 꼴(숫자1 · 숫자2 …)이 된다. 한 마디는 끝 점이 있어야 한다("2026 년"과 가르기)
static NUM_DOTTED: Lazy<Regex> = Lazy::new(|| Regex::new(r"^(\d{1,2}(?:\.\d{1,2}){0,5})(\.?)\s*[^\d\s.]").unwrap());
const NUM_DEPTH_FORMS: [&str; 6] = ["숫자1", "숫자2", "숫자3", "숫자4", "숫자5", "숫자6"];
// OCR 목차 줄 — 글 뒤에 공백 하나 + 쪽 번호(OCR 줄 묶기는 점선 · 긴 공백을 공백 하나로 만든다)
static TOC_OCR: Lazy<Regex> = Lazy::new(|| Regex::new(r"[가-힣A-Za-z)][^\d]*\s\d{1,3}$").unwrap());
// 그림 · 표 설명 — "그림 1.5:" "[그림 2]" "<표 3>" "표 2-1." 으로 시작하는 줄
static CAPTION_HEAD: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[\[<〈(]?\s*(?:그림|표|Figure|Fig\.|Table)\s*[\dA-Z]+(?:[.\-]\d+)*\s*[\]>〉):.]?").unwrap()
});
// 목차로 보려면 한 쪽에 목차 줄이 이만큼 이상
const MIN_TOC_ROWS: usize = 3;
// 제목은 대개 서술문으로 끝나지 않는다 — '…다.' '…다' 로 끝나는 줄은 제목에서 뺀다
//   ('개요' 처럼 '요'로 끝나는 제목 낱말이 많아 '요'는 '어요 · 해요 · 세요'만, '함 · 음 · 임'은 마침표가 있을 때만)
static SENTENCE_END: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:다\s*[.。]?|(?:어|해|세)요\s*[.。]?|(?:함|음|임)\s*[.。])\s*$").unwrap());
// 번호만 든 상자 — "1." "1.2" "8.5" "B.5" "Ⅱ." "가." "제3장"
static ENUM_ONLY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?:\d{1,2}(?:\.\d{1,2})*\.?|[A-Z]\.\d{1,2}(?:\.\d{1,2})*\.?|[ⅠⅡⅢⅣⅤⅥⅦⅧⅨⅩ]+\.?|[가나다라마바사아자차카타파하]\.|제\s*\d+\s*[편장절])$").unwrap()
});
// 줄 머리 번호(제목 글 비율을 잴 때 뺀다)
static LEAD_NUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[\s\d.()\-ⅠⅡⅢⅣⅤⅥⅦⅧⅨⅩ]+").unwrap());
static HANGUL_OR_LATIN: Lazy<Regex> = Lazy::new(|| Regex::new(r"[가-힣A-Za-z]").unwrap());
static TOC_TAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:\.{3,}|·{3,}|…{2,}|-{4,})\s*\d{1,4}\s*$").unwrap());

//------------------------------------------------------------------
// 쪽 하나의 입력
// -필드: no    = 쪽 번호(1부터)
// -필드: lines = OCR 줄 (글, [x0, y0, x1, y1]) — 200dpi 그림 픽셀
// -필드: size  = 쪽 크기 [가로, 세로] 픽셀(모르면 None — 상자 바깥 범위로 대신한다)
//------------------------------------------------------------------
pub struct PageIn {
    pub no: usize,
    pub lines: Vec<(String, [f32; 4])>,
    pub size: Option<[f32; 2]>,
}

//------------------------------------------------------------------
// 분석 중인 화면 줄 하나
// -필드: text      = 줄 글
// -필드: b         = 줄 범위 [x0, y0, x1, y1]
// -필드: h         = 상자 평균 높이
// -필드: chunk     = 속한 덩어리(단 조각) 번호 — 문단은 같은 덩어리 안에서만 묶는다
// -필드: table     = 표 행처럼 보임(상자 여럿 + 넓은 틈)
// -필드: kind      = 정한 라벨
// -필드: level     = 제목 수준 · 목록 깊이
// -필드: num_form  = 번호 제목 꼴(수준 정하기용)
//------------------------------------------------------------------
#[derive(Debug, Clone)]
struct Row {
    text: String,
    b: [f32; 4],
    h: f32,
    chunk: usize,
    table: bool,
    kind: &'static str,
    level: i64,
    num_form: Option<&'static str>,
}

// 정렬된 값의 중앙값(빈 목록이면 None)
fn median(v: &mut [f32]) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = v.len();
    Some(if m % 2 == 1 { v[m / 2] } else { (v[m / 2 - 1] + v[m / 2]) / 2.0 })
}

//------------------------------------------------------------------
// 단 나누기 (재귀 XY-cut)
//=> 상자들을 '같이 줄 묶기 할 덩어리' 목록으로 나눈다. 읽는 순서대로 돌려준다.
//    1) 가로축에 상자를 쌓아 거의 아무 상자도 지나지 않는 세로 빈 띠를 찾는다
//    2) 빈 띠 왼쪽 · 오른쪽에 글줄이 충분하고 나란히 있으면 단으로 본다(아니면 나누지 않음)
//    3) 빈 띠를 가로지르는 상자(제목 · 단 위아래 문단)는 '가로 상자'로 두고,
//       가로 상자 사이 구간마다 [왼쪽 단 → 오른쪽 단] 순으로 내보낸다
//    4) 각 단 안에서 한 번 더 나눠 본다(3단 문서)
//
// -in: lines = 쪽의 OCR 줄 전체
// -in: ids   = 이번에 나눌 상자 번호들
// -in: hm    = 쪽 상자 높이 중앙값
// -in: depth = 더 나눌 수 있는 깊이
//
// -out: 덩어리 목록(상자 번호들) — 나누지 못하면 [ids] 한 개
// -out: error = 없음
//------------------------------------------------------------------
fn split_columns(lines: &[(String, [f32; 4])], ids: Vec<usize>, hm: f32, depth: usize) -> Vec<Vec<usize>> {
    if depth == 0 || ids.len() < 2 * MIN_COL_BOXES || hm <= 0.0 {
        return vec![ids];
    }
    let gutter = match find_gutter(lines, &ids, hm) {
        Some(g) => g,
        None => return vec![ids],
    };
    let (left, right, mut span) = sides(lines, &ids, gutter, hm);
    // 가로 상자를 세로 가운데 순으로 — 그 사이 구간이 단 구간이다
    let yc = |i: usize| (lines[i].1[1] + lines[i].1[3]) / 2.0;
    span.sort_by(|&a, &b| yc(a).partial_cmp(&yc(b)).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<Vec<usize>> = vec![];
    let mut taken_l = vec![false; left.len()];
    let mut taken_r = vec![false; right.len()];
    let mut run: Vec<usize> = vec![];
    // 가로 상자 s 마다: 그 위에 남은 단 상자가 있으면 [쌓인 가로 상자 → 왼쪽 단 → 오른쪽 단] 순으로 내보낸다
    for &s in span.iter().chain(std::iter::once(&usize::MAX)) {
        let upto = if s == usize::MAX { f32::MAX } else { yc(s) };
        let mut segs: Vec<Vec<usize>> = vec![];
        for (side, taken) in [(&left, &mut taken_l), (&right, &mut taken_r)] {
            let mut seg = vec![];
            for (k, &i) in side.iter().enumerate() {
                if !taken[k] && yc(i) < upto {
                    taken[k] = true;
                    seg.push(i);
                }
            }
            segs.push(seg);
        }
        if segs.iter().any(|g| !g.is_empty()) {
            if !run.is_empty() {
                out.push(std::mem::take(&mut run));
            }
            for g in segs {
                if !g.is_empty() {
                    out.extend(split_columns(lines, g, hm, depth - 1));
                }
            }
        }
        if s != usize::MAX {
            run.push(s);
        }
    }
    if !run.is_empty() {
        out.push(run);
    }
    out.retain(|c| !c.is_empty());
    out
}

//------------------------------------------------------------------
// 빈 띠 기준으로 상자를 왼쪽 · 오른쪽 · 가로지름으로 나누기
//=> 빈 띠 가운데(mid)를 기준선으로 쓴다. 빈 띠 가장자리는 긴 줄 몇 개가 살짝 넘어
//   흐릿하므로, 가운데를 hm 넘게 넘어가는 상자만 '가로지름'(제목 · 쪽 폭 문단)으로 본다.
//
// -in: lines · ids = 쪽 줄과 볼 상자 / gutter = (빈 띠 왼쪽, 오른쪽) / hm = 상자 높이 중앙값
//
// -out: (왼쪽, 오른쪽, 가로지름) 상자 번호들
// -out: error = 없음
//------------------------------------------------------------------
fn sides(lines: &[(String, [f32; 4])], ids: &[usize], gutter: (f32, f32), hm: f32) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
    let mid = (gutter.0 + gutter.1) / 2.0;
    let (mut l, mut r, mut s) = (vec![], vec![], vec![]);
    for &i in ids {
        let b = lines[i].1;
        if b[0] < mid - hm && b[2] > mid + hm {
            s.push(i);
        } else if (b[0] + b[2]) / 2.0 < mid {
            l.push(i);
        } else {
            r.push(i);
        }
    }
    (l, r, s)
}

//------------------------------------------------------------------
// 세로 빈 띠(gutter) 찾기
//=> 상자 x 범위를 칸(hm/4)마다 세어, 지나는 상자가 거의 없는 칸이 이어진 곳을 후보로 본다.
//   후보마다 두 단 조건(양쪽 상자 수 · 세로 겹침 · 글줄이 단을 채움 · 글자 수)을 보고
//   통과한 것 가운데 가장 넓은 것을 고른다.
//
// -in: lines = 쪽의 OCR 줄 / ids = 볼 상자 번호들 / hm = 상자 높이 중앙값
//
// -out: Some((빈 띠 왼쪽 x, 오른쪽 x)) / None = 단 없음
// -out: error = 없음
//------------------------------------------------------------------
fn find_gutter(lines: &[(String, [f32; 4])], ids: &[usize], hm: f32) -> Option<(f32, f32)> {
    let x0 = ids.iter().map(|&i| lines[i].1[0]).fold(f32::MAX, f32::min);
    let x1 = ids.iter().map(|&i| lines[i].1[2]).fold(f32::MIN, f32::max);
    let w = x1 - x0;
    if w <= 0.0 {
        return None;
    }
    let bin = (hm / 4.0).max(1.0);
    let nb = ((w / bin).ceil() as usize).max(1);
    let mut cnt = vec![0usize; nb];
    for &i in ids {
        let b = lines[i].1;
        let a = (((b[0] - x0) / bin).floor() as usize).min(nb - 1);
        let z = ((((b[2] - x0) / bin).ceil() as usize).max(a + 1)).min(nb);
        for c in &mut cnt[a..z] {
            *c += 1;
        }
    }
    let allow = ((ids.len() as f32 * GUTTER_CROSS).floor() as usize).max(1);
    // 지나는 상자가 allow 이하인 칸이 이어진 구간들
    let mut runs: Vec<(usize, usize)> = vec![];
    let mut k = 0;
    while k < nb {
        if cnt[k] <= allow {
            let s = k;
            while k < nb && cnt[k] <= allow {
                k += 1;
            }
            runs.push((s, k));
        } else {
            k += 1;
        }
    }
    let mut best: Option<(f32, (f32, f32))> = None;
    for (s, e) in runs {
        // 구간 안에서 가장 적게 지나는 칸 수로 범위를 좁힌다(제목이 지나는 칸은 그대로 허용)
        let lo = x0 + s as f32 * bin;
        let hi = x0 + e as f32 * bin;
        let width = hi - lo;
        let mid = ((lo + hi) / 2.0 - x0) / w;
        if width < GUTTER_MIN * hm || mid < GUTTER_POS.0 || mid > GUTTER_POS.1 {
            continue;
        }
        if !columns_ok(lines, ids, lo, hi, hm) {
            continue;
        }
        if best.map_or(true, |(bw, _)| width > bw) {
            best = Some((width, (lo, hi)));
        }
    }
    best.map(|(_, g)| g)
}

//------------------------------------------------------------------
// 빈 띠 양쪽이 정말 나란한 글 단인가
//=> 1) 양쪽 상자가 MIN_COL_BOXES 개 이상
//   2) 양쪽 세로 범위가 짧은 쪽의 COL_OVERLAP 이상 겹침
//   3) 양쪽 모두 상자 너비 중앙값이 단 너비의 COL_FILL 이상(글줄이 단을 채움 — 표 칸과 다름)
//   4) 양쪽 모두 상자 글자 수 중앙값이 COL_MIN_CHARS 이상
//
// -in: lines · ids = 쪽 줄과 볼 상자 / lo · hi = 빈 띠 범위 / hm = 상자 높이 중앙값
//
// -out: true = 단으로 나눠도 된다
// -out: error = 없음
//------------------------------------------------------------------
fn columns_ok(lines: &[(String, [f32; 4])], ids: &[usize], lo: f32, hi: f32, hm: f32) -> bool {
    let (left, right, _) = sides(lines, ids, (lo, hi), hm);
    if left.len() < MIN_COL_BOXES || right.len() < MIN_COL_BOXES {
        return false;
    }
    let yr = |v: &[usize]| (v.iter().map(|&i| lines[i].1[1]).fold(f32::MAX, f32::min),
                            v.iter().map(|&i| lines[i].1[3]).fold(f32::MIN, f32::max));
    let (ly0, ly1) = yr(&left);
    let (ry0, ry1) = yr(&right);
    let ov = ly1.min(ry1) - ly0.max(ry0);
    if ov < COL_OVERLAP * (ly1 - ly0).min(ry1 - ry0) {
        return false;
    }
    // 한쪽 단이 글줄로 차 있나
    let filled = |v: &[usize]| -> bool {
        let cx0 = v.iter().map(|&i| lines[i].1[0]).fold(f32::MAX, f32::min);
        let cx1 = v.iter().map(|&i| lines[i].1[2]).fold(f32::MIN, f32::max);
        let mut ws: Vec<f32> = v.iter().map(|&i| lines[i].1[2] - lines[i].1[0]).collect();
        let mut cs: Vec<f32> = v.iter().map(|&i| pyutil::clen(pyutil::strip(&lines[i].0)) as f32).collect();
        let mw = median(&mut ws).unwrap_or(0.0);
        let mc = median(&mut cs).unwrap_or(0.0);
        mw >= COL_FILL * (cx1 - cx0) && mc >= COL_MIN_CHARS as f32
    };
    filled(&left) && filled(&right)
}

//------------------------------------------------------------------
// 본문 글자 높이 (글자 수 가중 최빈값)
//=> 15자 이상 줄의 높이를 2px 단위로 묶어 글자 수를 더하고, 가장 무거운 높이를 본문으로 본다.
//   PDF 제목 추론의 '가중 최빈 크기'와 같은 생각이다.
//
// -in: rows = 화면 줄들
//
// -out: (Some(본문 높이) 또는 None, 쓴 줄 수)
// -out: error = 없음
//------------------------------------------------------------------
fn body_height(rows: &[Row]) -> (Option<f32>, usize) {
    let mut weight: HashMap<i64, usize> = HashMap::new();
    let mut n = 0;
    for r in rows {
        // 글자(한글 · 영문 등)만 센다 — 화면 캡처 속 숫자 · 16진 문자열 줄이 본문 높이를 끌어내리지 않게
        let c = r.text.chars().filter(|ch| ch.is_alphabetic()).count();
        if c >= BODY_MIN_CHARS && !r.table {
            *weight.entry((r.h / 2.0).round() as i64).or_default() += c;
            n += 1;
        }
    }
    let best = weight.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|(k, _)| k as f32 * 2.0);
    (best, n)
}

//------------------------------------------------------------------
// 쪽 하나 → 화면 줄들 (단 나누기 + 줄 묶기)
//=> 단마다 group_rows 와 같은 규칙으로 묶고, 상자 사이 넓은 틈 수로 표 행 표시를 한다.
//
// -in: p = 쪽 입력
// -in: chunk_base = 덩어리 번호 시작값(쪽마다 다른 번호가 되게)
//
// -out: (읽는 순서의 줄 목록, 다음 덩어리 번호)
// -out: error = 없음
//------------------------------------------------------------------
fn page_rows(p: &PageIn, chunk_base: usize) -> (Vec<Row>, usize) {
    let ids: Vec<usize> = (0..p.lines.len()).filter(|&i| !pyutil::strip(&p.lines[i].0).is_empty()).collect();
    let mut hs: Vec<f32> = ids.iter().map(|&i| p.lines[i].1[3] - p.lines[i].1[1]).collect();
    let hm = median(&mut hs).unwrap_or(0.0);
    let chunks = split_columns(&p.lines, ids, hm, MAX_SPLIT_DEPTH);
    let mut rows = vec![];
    let mut cid = chunk_base;
    for ch in chunks {
        let sub: Vec<(String, [f32; 4])> = ch.iter().map(|&i| p.lines[i].clone()).collect();
        for (text, b, h, bids) in super::group_rows_ids(&sub) {
            let t = pyutil::strip(&text).to_string();
            if t.is_empty() {
                continue;
            }
            // 상자 사이 틈 — 넓은 틈이 여럿이면 표 행(칸 사이가 벌어져 있음)
            let mut wide = 0;
            for w in bids.windows(2) {
                if sub[w[1]].1[0] - sub[w[0]].1[2] >= TABLE_GAP * h.max(1.0) {
                    wide += 1;
                }
            }
            // 번호 상자 + 제목 상자 두 개("8.5      다중 보호프로파일")는 표 행이 아니다
            let numbered_pair = bids.len() == 2 && ENUM_ONLY.is_match(pyutil::strip(&sub[bids[0]].0));
            let table = bids.len() >= TABLE_MIN_BOXES && wide >= TABLE_MIN_GAPS && !numbered_pair;
            rows.push(Row { text: t, b, h, chunk: cid, table, kind: B::PARAGRAPH, level: 0, num_form: None });
        }
        cid += 1;
    }
    (rows, cid)
}

// 번호 제목 꼴 — 맞는 꼴 이름(없으면 None)
fn num_form(t: &str) -> Option<&'static str> {
    if structure::GRAMMARS.iter().any(|(name, pat)| *name == "조문" && structure::grammar_fullmatch(pat, t)) {
        return Some("조문");
    }
    if structure::appendix_fullmatch(t) {
        return Some("부속");
    }
    if let Some(f) = NUM_FORMS.iter().find(|(_, re)| re.is_match(t)).map(|(k, _)| *k) {
        return Some(f);
    }
    let c = NUM_DOTTED.captures(t)?;
    let depth = c[1].split('.').count();
    // 한 마디 숫자는 끝 점이 있어야 번호다("3 소매금융" · "2026 년" 은 아님)
    if depth == 1 && c[2].is_empty() {
        return None;
    }
    Some(NUM_DEPTH_FORMS[depth - 1])
}

//------------------------------------------------------------------
// 제목 글로 볼 만한가 (높이 · 번호 제목 공통)
//=> 1) 한글 · 영문이 공백 뺀 글자의 절반 이상(기호 · 숫자 잡음 "∗∗∗ S ***" 는 아님)
//   2) 같은 글자만 되풀이한 것이 아님("XOXXXX" 같은 표 체크 표시)
//   3) 서술문으로 끝나지 않음("…이다." "…된다")
//
// -in: t = 줄 글
//
// -out: true = 제목 후보가 될 수 있다
// -out: error = 없음
//------------------------------------------------------------------
fn heading_text_ok(t: &str) -> bool {
    // 앞 번호("1.2.3.1" "Ⅱ." "(3)")는 빼고 본다 — 번호 글자가 글자 비율을 끌어내리지 않게
    let t = LEAD_NUM.replace(t, "");
    let t = t.as_ref();
    let cs: Vec<char> = t.chars().filter(|c| !c.is_whitespace()).collect();
    if cs.is_empty() {
        return false;
    }
    let letters = cs.iter().filter(|c| ('가'..='힣').contains(c) || c.is_ascii_alphabetic()).count();
    if (letters as f32) < cs.len() as f32 * 0.5 {
        return false;
    }
    // 서로 다른 글자가 둘 이하인 영문 줄(체크 표시 X · O)은 제목이 아니다
    let mut kinds: Vec<char> = cs.iter().copied().filter(|c| c.is_ascii_alphabetic()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    if letters == cs.iter().filter(|c| c.is_ascii_alphabetic()).count() && kinds.len() <= 2 {
        return false;
    }
    !SENTENCE_END.is_match(t)
}

//------------------------------------------------------------------
// 앞 줄에 바로 이어지는 줄인가 (문단 가운데 · 끝 줄)
//=> 같은 덩어리의 바로 앞 줄이 가깝고(틈 ≤ PARA_GAP·h) 오른쪽 끝까지 차 있으면,
//   이 줄은 그 문단이 줄바꿈되어 이어진 것이다 — 키가 커 보여도 제목이 아니다.
//
// -in: rows = 쪽의 줄들 / ri = 볼 줄 / h = 본문 높이 / right = 덩어리별 오른쪽 끝
//
// -out: true = 앞 문단에 이어지는 줄
// -out: error = 없음
//------------------------------------------------------------------
fn continues_prev(rows: &[Row], ri: usize, h: f32, right: &HashMap<usize, f32>) -> bool {
    let r = &rows[ri];
    match ri.checked_sub(1).map(|k| &rows[k]) {
        Some(prev) if prev.chunk == r.chunk && !prev.table && prev.kind != B::HEADING => {
            let rr = right.get(&r.chunk).copied().unwrap_or(prev.b[2]);
            r.b[1] - prev.b[3] <= PARA_GAP * h && prev.b[2] >= rr - PARA_RIGHT_TOL * h
        }
        _ => false,
    }
}

//------------------------------------------------------------------
// 번호 꼴이 같은 제목은 같은 수준으로
//=> 높이 제목 수준은 상자 높이로 정해 ±10% 흔들림에 쪽마다 달라질 수 있다("1." 이 1쪽 수준 2, 2쪽 수준 1).
//    1) 번호 꼴(숫자1 · 숫자2 · 가나다 …)마다 문서에서 가장 높은(작은) 수준으로 맞춘다
//    2) 숫자 계열은 깊은 번호가 얕은 번호보다 반드시 아래: 숫자(d+1) 수준 = 숫자d 수준 + 1 이상
//
// -in: all = 쪽별 줄(제목 수준을 바꾼다)
//
// -out: 없음
// -out: error = 없음
//------------------------------------------------------------------
fn unify_levels(all: &mut [Vec<Row>]) {
    let mut lv_of: HashMap<&'static str, i64> = HashMap::new();
    for r in all.iter().flatten().filter(|r| r.kind == B::HEADING) {
        if let Some(f) = num_form(&r.text) {
            let e = lv_of.entry(f).or_insert(r.level);
            *e = (*e).min(r.level);
        }
    }
    // 숫자 계열 깊이 순서 지키기
    let mut prev: Option<i64> = None;
    for f in NUM_DEPTH_FORMS {
        if let Some(lv) = lv_of.get_mut(f) {
            if let Some(p) = prev {
                if *lv <= p {
                    *lv = (p + 1).min(MAX_LEVEL);
                }
            }
            prev = Some(*lv);
        }
    }
    for r in all.iter_mut().flatten().filter(|r| r.kind == B::HEADING) {
        if let Some(lv) = num_form(&r.text).and_then(|f| lv_of.get(f)) {
            r.level = *lv;
        }
    }
}

//------------------------------------------------------------------
// 높이 제목 뒤에 본문이 오나
//=> 제목은 본문 앞에 온다. 읽는 순서로 다음 몇 줄 안에, 가까이(HEAD_BODY_WITHIN·h) 본문 높이의
//   글줄(글자 10자 이상)이 있어야 한다. 화면 캡처 · 도식 속 큰 버튼 글("검색" "추가")은 뒤에 본문이 없다.
//
// -in: rows = 쪽의 줄들 / ri = 볼 줄 / h = 본문 높이
//
// -out: true = 뒤에 본문이 있다
// -out: error = 없음
//------------------------------------------------------------------
fn followed_by_body(rows: &[Row], ri: usize, h: f32) -> bool {
    let r = &rows[ri];
    rows.iter().skip(ri + 1).take(HEAD_BODY_LOOKAHEAD).any(|n| {
        let letters = n.text.chars().filter(|c| c.is_alphabetic()).count();
        let hr = n.h / h;
        !n.table && letters >= BODY_MIN_CHARS && hr >= PARA_H_RATIO.0 && hr <= PARA_H_RATIO.1
            && n.b[1] - r.b[3] <= HEAD_BODY_WITHIN * h
    })
}

// 머리말 비교용 글 — 공백 · 기호를 빼고 숫자는 '#'
fn edge_key(t: &str) -> Vec<char> {
    t.chars().filter(|c| c.is_alphanumeric()).map(|c| if c.is_ascii_digit() { '#' } else { c }).collect()
}

// 두 글의 바이그램 Dice 유사도(0~1) — 2 × 겹친 바이그램 수 / (양쪽 바이그램 수 합)
fn dice(a: &[char], b: &[char]) -> f32 {
    if a.len() < 2 || b.len() < 2 {
        return if a == b { 1.0 } else { 0.0 };
    }
    let ga: Vec<(char, char)> = a.windows(2).map(|w| (w[0], w[1])).collect();
    let mut gb: Vec<(char, char)> = b.windows(2).map(|w| (w[0], w[1])).collect();
    let total = ga.len() + gb.len();
    let mut hit = 0;
    for g in &ga {
        // 같은 바이그램을 두 번 세지 않게 짝지은 것은 지운다
        if let Some(k) = gb.iter().position(|x| x == g) {
            gb.swap_remove(k);
            hit += 1;
        }
    }
    2.0 * hit as f32 / total as f32
}

//------------------------------------------------------------------
// 머리말 · 꼬리말 (OCR 글 흔들림 허용)
//=> edge_kinds 는 글이 똑같아야 되풀이로 본다. OCR 은 쪽마다 "(AI)" ↔ "(A)", 띄어쓰기가 흔들려
//   같은 머리말을 놓친다. 그래서 아직 문단인 가장자리 줄끼리 바이그램 유사도로 다시 묶는다.
//    1) 쪽마다 위 · 아래 EDGE_ROWS 줄 중 쪽 가장자리 구역 안 · 60자 이하 · 글자 4개 이상
//    2) 유사도 ≥ EDGE_SIM 이면 같은 머리말 묶음
//    3) 묶음이 MIN_EDGE_PAGES 쪽 이상이면 첫 쪽 것을 빼고 header(위) · footer(아래)
//       (첫 것을 남기는 것은 PDF edge_kinds 와 같은 규칙)
//
// -in: all = 쪽별 줄(라벨을 바꾼다) / ysorted = 쪽별 위→아래 줄 순서 / pages = 쪽 입력(크기)
//
// -out: 없음 (all 의 kind 를 바꾼다)
// -out: error = 없음
//------------------------------------------------------------------
fn fuzzy_edges(all: &mut [Vec<Row>], ysorted: &[Vec<usize>], pages: &[PageIn]) {
    // (쪽, 줄, 위인가, 글 키)
    let mut cands: Vec<(usize, usize, bool, Vec<char>)> = vec![];
    for (pi, rows) in all.iter().enumerate() {
        if rows.is_empty() {
            continue;
        }
        let (top, bot) = match pages[pi].size {
            Some(s) => (0.0, s[1]),
            None => (rows.iter().map(|r| r.b[1]).fold(f32::MAX, f32::min), rows.iter().map(|r| r.b[3]).fold(f32::MIN, f32::max)),
        };
        let zone = (bot - top) * EDGE_ZONE;
        let ix = &ysorted[pi];
        let n = ix.len();
        let picks: Vec<(usize, bool)> = ix.iter().take(EDGE_ROWS).map(|&i| (i, true))
            .chain(ix.iter().skip(n.saturating_sub(EDGE_ROWS)).map(|&i| (i, false))).collect();
        for (ri, is_top) in picks {
            let r = &rows[ri];
            let in_zone = if is_top { r.b[3] <= top + zone } else { r.b[1] >= bot - zone };
            let key = edge_key(&r.text);
            let letters = key.iter().filter(|c| **c != '#').count();
            if r.kind == B::PARAGRAPH && in_zone && pyutil::clen(&r.text) <= HEAD_MAX_CHARS && letters >= 4 {
                cands.push((pi, ri, is_top, key));
            }
        }
    }
    let mut used = vec![false; cands.len()];
    for a in 0..cands.len() {
        if used[a] {
            continue;
        }
        let mut group = vec![a];
        for b in a + 1..cands.len() {
            if !used[b] && cands[b].2 == cands[a].2 && cands[b].0 != cands[a].0 && dice(&cands[a].3, &cands[b].3) >= EDGE_SIM {
                group.push(b);
            }
        }
        let mut pgs: Vec<usize> = group.iter().map(|&g| cands[g].0).collect();
        pgs.sort_unstable();
        pgs.dedup();
        if pgs.len() < MIN_EDGE_PAGES {
            continue;
        }
        for &g in &group {
            used[g] = true;
        }
        // 첫 쪽 것은 문단으로 남긴다(PDF 규칙과 같음)
        for &g in group.iter().skip(1) {
            let (pi, ri, is_top, _) = &cands[g];
            all[*pi][*ri].kind = if *is_top { B::HEADER } else { B::FOOTER };
        }
    }
}

// 줄이 덩어리 오른쪽 끝까지 찼나 — 제목은 짧고, 끝까지 찬 줄은 본문이다
//   (OCR 상자 높이는 ±15% 흔들려 본문 줄도 키 큰 줄로 보일 때가 있다 — 실제 다단 문서 mc07 측정)
fn full_width(r: &Row, h: f32, right: &HashMap<usize, f32>) -> bool {
    right.get(&r.chunk).map_or(false, |&rr| r.b[2] >= rr - PARA_RIGHT_TOL * h)
}

// 덩어리마다 오른쪽 끝(줄 x1 의 상위 90%) — 표 행은 빼고 잰다
fn chunk_right(rows: &[Row]) -> HashMap<usize, f32> {
    let mut by: HashMap<usize, Vec<f32>> = HashMap::new();
    for r in rows.iter().filter(|r| !r.table) {
        by.entry(r.chunk).or_default().push(r.b[2]);
    }
    by.into_iter().map(|(c, mut v)| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let k = ((v.len() as f32 * 0.9) as usize).min(v.len() - 1);
        (c, v[k])
    }).collect()
}

//------------------------------------------------------------------
// OCR 쪽들 → 구조 블록 (바깥 입구)
//=> 한 문서의 OCR 쪽들을 함께 받아야 머리말 되풀이 · 제목 안전장치를 문서 단위로 볼 수 있다.
//    1) 쪽마다 단 나누기 + 줄 묶기
//    2) 쪽마다 본문 높이 h (모자라면 문서 전체 값)
//    3) 가장자리 줄 — pdf::edge_kinds(위아래 2줄 · 쪽번호 · 3쪽 되풀이) + 위치 조건
//    4) 높이 제목 · 번호 제목 · 목록 라벨
//    5) 같은 덩어리 안에서 문단 묶기 → 블록
//
// -in: pages = 쪽 입력들(쪽 번호 순)
//
// -out: 블록 목록 — 쪽 순, 쪽 안은 읽는 순서. 출처는 제목 inferred, 나머지 ocr
// -out: error = 없음
//------------------------------------------------------------------
pub fn analyze(pages: &[PageIn]) -> Vec<Block> {
    // 1) 줄 만들기
    let mut all: Vec<Vec<Row>> = vec![];
    let mut cid = 0;
    for p in pages {
        let (rows, next) = page_rows(p, cid);
        cid = next;
        all.push(rows);
    }
    // 2) 본문 높이 — 문서 전체 값은 모든 줄을 모아 같은 방법으로
    let flat: Vec<Row> = all.iter().flatten().cloned().collect();
    let doc_h = body_height(&flat).0.or_else(|| {
        let mut v: Vec<f32> = flat.iter().map(|r| r.h).collect();
        median(&mut v)
    }).unwrap_or(0.0);
    // 쪽마다 배율이 다를 수 있다(2쪽 모아 찍기 · 축소 인쇄) — 본문 줄이 충분한 쪽은 그 쪽 값, 아니면 문서 값
    let page_h: Vec<f32> = all.iter().map(|rows| match body_height(rows) {
        (Some(h), n) if n >= BODY_MIN_ROWS => h,
        _ => doc_h,
    }).collect();

    // 3) 가장자리 — 쪽마다 위에서 아래(y) 순서의 줄 글로 기존 규칙을 돌린다
    let mut ysorted: Vec<Vec<usize>> = vec![];
    for rows in &all {
        let mut ix: Vec<usize> = (0..rows.len()).collect();
        ix.sort_by(|&a, &b| rows[a].b[1].partial_cmp(&rows[b].b[1]).unwrap_or(std::cmp::Ordering::Equal));
        ysorted.push(ix);
    }
    let texts: Vec<Vec<String>> = all.iter().zip(&ysorted).map(|(rows, ix)| ix.iter().map(|&i| rows[i].text.clone()).collect()).collect();
    let edges = crate::docmodel::pdf::edge_kinds(&texts);
    for (pi, (rows, ix)) in all.iter_mut().zip(&ysorted).enumerate() {
        if rows.is_empty() {
            continue;
        }
        // 쪽 위아래 범위 — 쪽 크기를 알면 그것, 모르면 줄들의 바깥 범위
        let (top, bot) = match pages[pi].size {
            Some(s) => (0.0, s[1]),
            None => (rows.iter().map(|r| r.b[1]).fold(f32::MAX, f32::min), rows.iter().map(|r| r.b[3]).fold(f32::MIN, f32::max)),
        };
        let zone = (bot - top) * EDGE_ZONE;
        for (j, &ri) in ix.iter().enumerate() {
            if let Some(&k) = edges.get(&(pi as i64 + 1, j)) {
                let r = &mut rows[ri];
                // 위치 조건 — 정말 쪽 위 · 아래 가장자리에 있는 줄만
                if r.b[3] <= top + zone || r.b[1] >= bot - zone {
                    r.kind = k;
                }
            }
        }
    }
    fuzzy_edges(&mut all, &ysorted, pages);

    // 진단용: SDP_LAYOUT_DEBUG 가 있으면 줄마다 높이 비율 · 앞 줄 틈 · 표 표시를 stderr 로 찍는다
    if std::env::var_os("SDP_LAYOUT_DEBUG").is_some() {
        eprintln!("doc_h={}", doc_h);
        for (pi, rows) in all.iter().enumerate() {
            for (ri, r) in rows.iter().enumerate() {
                let gap = if ri > 0 { r.b[1] - rows[ri - 1].b[3] } else { -1.0 };
                eprintln!("p{} c{} h={:.0} r={:.2} gap={:.0} tab={} x0={:.0} x1={:.0} {}", pages[pi].no, r.chunk, r.h, r.h / doc_h.max(1.0), gap, r.table, r.b[0], r.b[2], r.text.chars().take(40).collect::<String>());
            }
        }
    }
    // 4-0) 목차 — 한 쪽에 '글 + 쪽 번호' 줄이 MIN_TOC_ROWS 개 이상이면 그 줄들을 목차로
    for rows in all.iter_mut() {
        // 목차 줄은 글과 쪽 번호 사이가 벌어져 표 행으로도 보인다 — 표 표시는 따지지 않는다
        let hits: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].kind == B::PARAGRAPH && TOC_OCR.is_match(&rows[i].text)).collect();
        if hits.len() >= MIN_TOC_ROWS {
            for i in hits {
                rows[i].kind = B::TOC;
            }
        }
    }
    // 4-1) 그림 · 표 설명
    for rows in all.iter_mut() {
        for r in rows.iter_mut() {
            if r.kind == B::PARAGRAPH && pyutil::clen(&r.text) <= HEAD_MAX_CHARS * 2 && CAPTION_HEAD.is_match(&r.text) {
                r.kind = B::CAPTION;
            }
        }
    }
    let rights: Vec<HashMap<usize, f32>> = all.iter().map(|rows| chunk_right(rows)).collect();

    // 4) 제목 · 목록
    let n_rows: usize = all.iter().map(|r| r.len()).sum();
    let mut size_cand: Vec<(usize, usize, f32)> = vec![];
    for (pi, rows) in all.iter().enumerate() {
        let h = page_h[pi];
        if h <= 0.0 {
            continue;
        }
        for (ri, r) in rows.iter().enumerate() {
            if r.kind != B::PARAGRAPH || r.table {
                continue;
            }
            let ratio = r.h / h;
            if ratio >= HEAD_RATIO && pyutil::clen(&r.text) <= HEAD_MAX_CHARS && HANGUL_OR_LATIN.is_match(&r.text) && !TOC_TAIL.is_match(&r.text)
                && heading_text_ok(&r.text) && !continues_prev(rows, ri, h, &rights[pi])
                // 아주 큰 줄은 쪽 제목 — 뒤에 표가 오거나 혼자 한 덩어리여도 제목으로 본다
                && (ratio >= HEAD_HUGE_RATIO || (followed_by_body(rows, ri, h) && !full_width(r, h, &rights[pi]))) {
                size_cand.push((pi, ri, ratio));
            }
        }
    }
    // 문서 단위 안전장치 — 너무 적거나(우연) 너무 많으면(배율 흔들림) 높이 제목을 쓰지 않는다
    let mut max_size_level = 0;
    // 후보가 적으면 아주 큰 것만 남긴다
    if size_cand.len() < MIN_SIZE_HEADS {
        size_cand.retain(|c| c.2 >= HEAD_SURE_RATIO);
    }
    if !size_cand.is_empty() && (size_cand.len() as f32) <= n_rows as f32 * HEAD_MAX_SHARE {
        // 비율을 큰 것부터 보고 LEVEL_STEP 넘게 떨어지면 다음 수준
        let mut ratios: Vec<f32> = size_cand.iter().map(|c| c.2).collect();
        ratios.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let mut cuts: Vec<f32> = vec![ratios[0]];
        for &r in &ratios[1..] {
            if cuts.last().unwrap() - r > LEVEL_STEP {
                cuts.push(r);
            }
        }
        for (pi, ri, ratio) in &size_cand {
            // 자기보다 큰 기준점 수 = 수준 - 1
            let lv = (cuts.iter().filter(|&&c| c - ratio > LEVEL_STEP / 2.0).count() as i64 + 1).min(MAX_SIZE_LEVEL);
            let r = &mut all[*pi][*ri];
            r.kind = B::HEADING;
            r.level = lv;
            max_size_level = max_size_level.max(lv);
        }
    }
    // 번호 제목 — 본문 높이 줄이라도 번호 꼴 + 짧음 + 앞 줄과 떨어짐
    if NUM_HEADINGS {
        let mut cand: Vec<(usize, usize, &'static str)> = vec![];
        for (pi, rows) in all.iter().enumerate() {
            let h = page_h[pi].max(1.0);
            for ri in 0..rows.len() {
                let r = &rows[ri];
                if r.kind != B::PARAGRAPH || r.table || pyutil::clen(&r.text) > NUM_HEAD_MAX_CHARS || !heading_text_ok(&r.text) {
                    continue;
                }
                let form = match num_form(&r.text) {
                    Some(f) => f,
                    None => continue,
                };
                // 같은 덩어리의 바로 앞 줄과 떨어져 있어야 한다(문단 첫 줄의 번호 목록과 가르기)
                let gap_ok = match ri.checked_sub(1).map(|k| &rows[k]) {
                    Some(prev) if prev.chunk == r.chunk => r.b[1] - prev.b[3] >= NUM_HEAD_GAP * h,
                    _ => true,
                };
                if gap_ok {
                    cand.push((pi, ri, form));
                }
            }
        }
        if !cand.is_empty() && (cand.len() as f32) <= n_rows as f32 * NUM_HEAD_MAX_SHARE {
            // 꼴이 처음 나온 순서가 수준 — 부속서 표지는 언제나 맨 위
            let mut order: Vec<&'static str> = vec![];
            for (_, _, f) in &cand {
                if *f != "부속" && !order.contains(f) {
                    order.push(f);
                }
            }
            // 높이 제목 중 번호 꼴이 있는 것을 기준점으로 — "1." 이 수준 1 이면 "1.1" 은 수준 2
            let mut anchor: HashMap<&'static str, i64> = HashMap::new();
            for rows in all.iter() {
                for r in rows.iter().filter(|r| r.kind == B::HEADING) {
                    if let Some(f) = num_form(&r.text) {
                        let e = anchor.entry(f).or_insert(r.level);
                        *e = (*e).min(r.level);
                    }
                }
            }
            // 숫자 깊이(숫자1 → 1)
            let depth = |f: &str| f.strip_prefix("숫자").and_then(|d| d.parse::<i64>().ok());
            for (pi, ri, f) in cand {
                let rank = if f == "부속" { 0 } else { order.iter().position(|o| *o == f).unwrap_or(0) as i64 };
                let by_anchor = anchor.get(f).copied().or_else(|| {
                    // 같은 숫자 계열 기준점이 있으면 깊이 차이만큼 아래
                    let d = depth(f)?;
                    anchor.iter().filter_map(|(k, lv)| depth(k).map(|dk| lv + (d - dk))).min()
                });
                let r = &mut all[pi][ri];
                r.kind = B::HEADING;
                r.level = by_anchor.unwrap_or(max_size_level + rank + 1).clamp(1, MAX_LEVEL);
                r.num_form = Some(f);
            }
        }
    }
    unify_levels(&mut all);
    // 목록 — 덩어리 안 들여쓰기 순위가 깊이
    for (pi, rows) in all.iter_mut().enumerate() {
        let h = page_h[pi].max(1.0);
        let mut by_chunk: HashMap<usize, Vec<usize>> = HashMap::new();
        for (ri, r) in rows.iter().enumerate() {
            if r.kind == B::PARAGRAPH && !r.table && LIST_HEAD.is_match(&r.text) {
                by_chunk.entry(r.chunk).or_default().push(ri);
            }
        }
        for (_, ris) in by_chunk {
            let mut xs: Vec<f32> = ris.iter().map(|&ri| rows[ri].b[0]).collect();
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            // h 보다 가까운 들여쓰기는 같은 깊이
            let mut steps: Vec<f32> = vec![];
            for x in xs {
                if steps.last().map_or(true, |&s| x - s > h) {
                    steps.push(x);
                }
            }
            for ri in ris {
                let x = rows[ri].b[0];
                let lv = steps.iter().filter(|&&s| x - s > -h / 2.0).count().max(1) as i64;
                rows[ri].kind = B::LIST_ITEM;
                rows[ri].level = lv;
            }
        }
    }

    // 5) 문단 묶기 → 블록
    let mut out = vec![];
    for (pi, rows) in all.iter().enumerate() {
        let no = Some(pages[pi].no as i64);
        let h = page_h[pi].max(1.0);
        // 덩어리마다 왼쪽 끝(최빈) · 오른쪽 끝(상위 90%)
        let mut edges_lr: HashMap<usize, (f32, f32)> = HashMap::new();
        let mut chunk_ids: Vec<usize> = rows.iter().map(|r| r.chunk).collect();
        chunk_ids.dedup();
        for c in chunk_ids {
            let rs: Vec<&Row> = rows.iter().filter(|r| r.chunk == c && !r.table).collect();
            if rs.is_empty() {
                continue;
            }
            let mut lw: HashMap<i64, usize> = HashMap::new();
            for r in &rs {
                *lw.entry((r.b[0] / (h / 2.0)).round() as i64).or_default() += 1;
            }
            let l = lw.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0))).map(|(k, _)| k as f32 * h / 2.0).unwrap_or(0.0);
            let mut x1s: Vec<f32> = rs.iter().map(|r| r.b[2]).collect();
            x1s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let r90 = x1s[((x1s.len() as f32 * 0.9) as usize).min(x1s.len() - 1)];
            edges_lr.insert(c, (l, r90));
        }
        // 지금 쌓는 블록: (종류, 수준, 글, 첫 줄, 마지막 줄, 줄 수)
        let mut cur: Option<(&'static str, i64, String, Row, Row, usize)> = None;
        for r in rows {
            let joined = match cur.as_mut() {
                Some((kind, lv, text, first, last, n)) if *n < PARA_MAX_ROWS && last.chunk == r.chunk && !r.table && !last.table
                    && (r.kind == B::PARAGRAPH || (r.kind == B::HEADING && *kind == B::HEADING && r.level == *lv)) => {
                    let (l, rr) = edges_lr.get(&r.chunk).copied().unwrap_or((r.b[0], r.b[2]));
                    let gap = r.b[1] - last.b[3];
                    let hr = r.h / last.h.max(1.0);
                    let full = last.b[2] >= rr - PARA_RIGHT_TOL * h;
                    let near = gap <= PARA_GAP * h && hr >= PARA_H_RATIO.0 && hr <= PARA_H_RATIO.1;
                    let ok = match *kind {
                        // 두 줄로 꺾인 제목 — 같은 수준 제목 줄이 바로 붙어 있으면 한 제목
                        k if k == B::HEADING => r.kind == B::HEADING && near,
                        _ if r.kind != B::PARAGRAPH => false,
                        // 문단 이어지는 줄 — 왼쪽 끝에 맞춰 시작
                        k if k == B::PARAGRAPH => near && full && (r.b[0] - l).abs() <= PARA_LEFT_TOL * h,
                        // 목록 항목 이어지는 줄 — 기호보다 들여 써서 시작
                        k if k == B::LIST_ITEM => near && full && r.b[0] > first.b[0] + 0.5 * h && r.b[0] <= first.b[0] + 4.0 * h,
                        _ => false,
                    };
                    if ok {
                        // 영문 하이픈 줄바꿈만 붙여 쓰고, 나머지는 공백 하나(한국어 줄바꿈은 대개 어절 경계)
                        let hyphen = text.ends_with('-') && text.chars().rev().nth(1).map_or(false, |c| c.is_ascii_alphabetic());
                        if !hyphen {
                            text.push(' ');
                        }
                        text.push_str(&r.text);
                        *last = r.clone();
                        *n += 1;
                    }
                    ok
                }
                _ => false,
            };
            if joined {
                continue;
            }
            if let Some((kind, lv, text, _, _, _)) = cur.take() {
                out.push(make_block(kind, lv, text, no));
            }
            cur = Some((r.kind, r.level, r.text.clone(), r.clone(), r.clone(), 1));
        }
        if let Some((kind, lv, text, _, _, _)) = cur.take() {
            out.push(make_block(kind, lv, text, no));
        }
    }
    out
}

// 블록 하나 만들기 — 제목은 추론(inferred), 나머지는 OCR 출처
fn make_block(kind: &'static str, level: i64, text: String, page: Option<i64>) -> Block {
    let src = if kind == B::HEADING { B::SRC_INFERRED } else { B::SRC_OCR };
    Block::new(kind, text).lv(level).pg(page).src(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 상자 하나 — (글, x0, y0, 너비, 높이)
    fn bx(t: &str, x: f32, y: f32, w: f32, h: f32) -> (String, [f32; 4]) {
        (t.to_string(), [x, y, x + w, y + h])
    }

    // 본문 줄 n 개를 (x, y) 부터 줄 간격 50 으로
    fn body(lines: &mut Vec<(String, [f32; 4])>, x: f32, y: f32, w: f32, n: usize, tag: &str) {
        for k in 0..n {
            lines.push(bx(&format!("{}{} 본문 글줄이 이어지는 문장입니다 가나다라", tag, k), x, y + k as f32 * 50.0, w, 40.0));
        }
    }

    fn page(no: usize, lines: Vec<(String, [f32; 4])>) -> PageIn {
        PageIn { no, lines, size: Some([1700.0, 2200.0]) }
    }

    #[test]
    fn two_columns_read_left_then_right() {
        let mut l = vec![];
        body(&mut l, 100.0, 300.0, 700.0, 6, "왼");
        body(&mut l, 900.0, 300.0, 700.0, 6, "오");
        let blocks = analyze(&[page(1, l)]);
        let text: Vec<&str> = blocks.iter().map(|b| b.text.as_str()).collect();
        let joined = text.join("|");
        // 왼쪽 단 글이 모두 오른쪽 단 글보다 앞
        let last_left = joined.rfind("왼").unwrap();
        let first_right = joined.find("오").unwrap();
        assert!(last_left < first_right, "{}", joined);
        // 한 줄에 두 단이 섞이지 않는다
        assert!(text.iter().all(|t| !(t.contains("왼") && t.contains("오"))));
    }

    #[test]
    fn title_above_two_columns_comes_first() {
        let mut l = vec![bx("두 단 문서의 큰 제목", 500.0, 100.0, 700.0, 70.0)];
        body(&mut l, 100.0, 300.0, 700.0, 6, "왼");
        body(&mut l, 900.0, 300.0, 700.0, 6, "오");
        let blocks = analyze(&[page(1, l)]);
        assert_eq!(blocks[0].text, "두 단 문서의 큰 제목");
    }

    #[test]
    fn single_column_paragraph_merges_and_breaks_on_gap() {
        let mut l = vec![];
        body(&mut l, 100.0, 300.0, 1400.0, 3, "가");
        // 큰 틈 뒤 새 문단
        body(&mut l, 100.0, 600.0, 1400.0, 2, "나");
        let blocks = analyze(&[page(1, l)]);
        assert_eq!(blocks.len(), 2, "{:?}", blocks.iter().map(|b| &b.text).collect::<Vec<_>>());
        assert!(blocks.iter().all(|b| b.kind == B::PARAGRAPH));
    }

    #[test]
    fn tall_rows_become_headings_across_pages() {
        let mut ps = vec![];
        for no in 1..=3 {
            // 제목 글이 쪽마다 달라야 한다 — 같으면(숫자만 다르면) 머리말 되풀이로 본다
            let names = ["사업 개요", "추진 전략", "기대 효과"];
            let mut l = vec![bx(names[no - 1], 100.0, 100.0, 600.0, 60.0)];
            body(&mut l, 100.0, 250.0, 1400.0, 8, "본");
            ps.push(page(no, l));
        }
        let blocks = analyze(&ps);
        let heads: Vec<&Block> = blocks.iter().filter(|b| b.kind == B::HEADING).collect();
        assert_eq!(heads.len(), 3);
        assert!(heads.iter().all(|b| b.level == 1 && b.source == B::SRC_INFERRED));
    }

    #[test]
    fn repeated_header_and_page_number() {
        let mut ps = vec![];
        for no in 1..=4 {
            let mut l = vec![bx("주식회사 예시 보고서 머리말", 100.0, 40.0, 600.0, 40.0)];
            body(&mut l, 100.0, 300.0, 1400.0, 5, "본");
            l.push(bx(&format!("- {} -", no), 800.0, 2100.0, 80.0, 40.0));
            ps.push(page(no, l));
        }
        let blocks = analyze(&ps);
        assert_eq!(blocks.iter().filter(|b| b.kind == B::PAGE_NUMBER).count(), 4);
        // 첫 쪽 머리말은 본문으로 남긴다(기존 edge_kinds 규칙과 같음)
        assert_eq!(blocks.iter().filter(|b| b.kind == B::HEADER).count(), 3);
    }

    #[test]
    fn ocr_noisy_header_still_repeats() {
        let heads = ["인공지능(AI)시대, 진화하는 은행업", "인공지능(A)시대,진화하는 은행업", "인공지능(AI) 시대,진화하는 은행업", "인공지능(AI)시대, 진화하는은행업"];
        let ps: Vec<PageIn> = heads.iter().enumerate().map(|(k, h)| {
            let mut l = vec![bx(h, 100.0, 40.0, 600.0, 40.0)];
            body(&mut l, 100.0, 300.0, 1400.0, 5, "본");
            page(k + 1, l)
        }).collect();
        let blocks = analyze(&ps);
        assert_eq!(blocks.iter().filter(|b| b.kind == B::HEADER).count(), 3);
    }

    #[test]
    fn list_items_and_table_rows() {
        let l = vec![
            bx("- 첫째 목록 항목입니다", 140.0, 300.0, 600.0, 40.0),
            bx("- 둘째 목록 항목입니다", 140.0, 350.0, 600.0, 40.0),
            // 표 행 — 상자 셋, 넓은 틈 둘. 번호 꼴이어도 제목이 되면 안 된다
            bx("3.", 100.0, 600.0, 40.0, 40.0),
            bx("소매금융여신심사부", 500.0, 600.0, 300.0, 40.0),
            bx("12", 1200.0, 600.0, 60.0, 40.0),
        ];
        let blocks = analyze(&[page(1, l)]);
        assert_eq!(blocks.iter().filter(|b| b.kind == B::LIST_ITEM).count(), 2);
        assert!(blocks.iter().all(|b| b.kind != B::HEADING));
    }

    #[test]
    fn numbered_heading_after_gap() {
        let mut l = vec![];
        body(&mut l, 100.0, 200.0, 1400.0, 4, "가");
        l.push(bx("1. 사업 개요", 100.0, 500.0, 300.0, 40.0));
        body(&mut l, 100.0, 560.0, 1400.0, 4, "나");
        l.push(bx("2. 추진 계획", 100.0, 860.0, 300.0, 40.0));
        body(&mut l, 100.0, 920.0, 1400.0, 4, "다");
        let blocks = analyze(&[page(1, l)]);
        let heads: Vec<&str> = blocks.iter().filter(|b| b.kind == B::HEADING).map(|b| b.text.as_str()).collect();
        assert_eq!(heads, vec!["1. 사업 개요", "2. 추진 계획"]);
    }
}
