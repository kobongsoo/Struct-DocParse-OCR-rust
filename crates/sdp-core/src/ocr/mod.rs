//------------------------------------------------------------------
// 스캔 문서·그림 파일 OCR (OCR 설계서 2026-11-13)
//=> 검출 PP-OCRv6_tiny_det + 인식 korean_PP-OCRv5_mobile_rec (둘 다 ONNX, onnxruntime 로 돌림).
//   RapidOCR 3.9.2 를 정답으로 두고 같은 결과가 나오게 옮겼다(OpenCV 없음).
//    · 엔진은 처음 쓸 때 한 번 적재하고, 대기열이 끝나면 unload() 로 내려 메모리를 돌려준다.
//    · 결과(줄 목록)는 파일 내용 해시로 캐시한다 — 재구축·이동·이름 바꿈에도 다시 OCR 하지 않는다.
//------------------------------------------------------------------

pub mod det;
pub mod geom;
pub mod image;
pub mod imgops;
pub mod layout;
pub mod orient;
pub mod rec;
pub mod run;

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config::cfg;
use crate::pyutil;
pub use imgops::Bgr;

// 모델·후처리가 바뀌면 올린다 — 옛 캐시는 무시하고 다시 OCR 한다
pub const ENGINE: &str = "ppocrv6-tiny-det+ko-ppocrv5-mobile-rec/1";
pub const DET_FILE: &str = "PP-OCRv6_tiny_det.onnx";
pub const REC_FILE: &str = "korean_PP-OCRv5_mobile_rec.onnx";
pub const DICT_FILE: &str = "korean_PP-OCRv5_mobile_rec.dict.txt";
// PDF 쪽을 그리는 해상도 — 측정(2026-11-13)도 200dpi 로 했다.
// 큰 쪽은 상한(CAP_LONG · CAP_SHORT) 때문에 더 낮게 그릴 수 있지만, 캐시 좌표는 늘 이 해상도 기준으로 되돌려 저장한다
pub const DPI: f32 = 200.0;
// 큰 쪽 · 큰 그림 상한 — A4 를 200dpi 로 그린 크기(긴 변 · 짧은 변 픽셀, OCR 속도개선 설계서 4장)
pub const CAP_LONG: f32 = 2339.0;
pub const CAP_SHORT: f32 = 1654.0;
// 이보다 덜 줄여도 되는 쪽(= A4 보다 5% 이내로 큰 쪽)은 줄이지 않는다.
// A4 를 pt 반올림 차이로 1픽셀 넘는 쪽(1655×2339)이나 US Letter(1700×2200, 0.97배)가 지금과 달라지지 않게
pub const CAP_MIN_SCALE: f32 = 0.95;
// 작은 쪽 확대 상한 — 실측(2026-11-17, A6 쪽 8쪽)에서 ×1.4 가 ×2.0 글자의 97% 를 시간 +31% 로 얻었다
pub const UP_MAX: f32 = 1.4;
// 이 배율 이하로만 키울 수 있는 쪽(= A4 보다 5% 안쪽으로 작은 쪽)은 키우지 않는다
pub const UP_MIN: f32 = 1.05;
// 줄 점수가 이보다 낮으면 버린다(RapidOCR text_score 기본값)
const TEXT_SCORE: f32 = 0.5;
// 그림 크기 한계(RapidOCR Global.max_side_len · min_side_len)
const MAX_SIDE: usize = 4000;
const MIN_SIDE: usize = 30;

//------------------------------------------------------------------
// OCR 한 줄 (검출 상자 하나)
// -필드: text  = 읽은 글
// -필드: score = 글자 확률 평균(0~1)
// -필드: quad  = 상자 네 꼭짓점 [왼위, 오른위, 오른아래, 왼아래] — 입력 그림 픽셀 좌표
//------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Line {
    pub text: String,
    pub score: f32,
    pub quad: [[f32; 2]; 4],
}

impl Line {
    // 상자를 감싸는 축 정렬 범위 (x0, y0, x1, y1)
    pub fn bbox(&self) -> [f32; 4] {
        let xs = self.quad.iter().map(|p| p[0]);
        let ys = self.quad.iter().map(|p| p[1]);
        [xs.clone().fold(f32::MAX, f32::min), ys.clone().fold(f32::MAX, f32::min),
         xs.fold(f32::MIN, f32::max), ys.fold(f32::MIN, f32::max)]
    }
}

//------------------------------------------------------------------
// 적재된 OCR 엔진
// -필드: det · rec = 검출 · 인식 모델 세션
// -필드: dict      = 글자 사전
// -필드: arena     = 세션을 만들 때의 아레나 설정 — 옵션이 바뀌면 다시 만든다
// -필드: threads   = 세션을 만들 때의 스레드 수 — 옵션이 바뀌면 다시 만든다
// -필드: ro        = 실행 옵션. 아레나가 켜져 있으면 "실행마다 줄이기" 항목이 들어 있다(실행마다 새로 만들지 않으려고 보관)
//------------------------------------------------------------------
struct Engine {
    det: ort::session::Session,
    rec: ort::session::Session,
    dict: Vec<String>,
    arena: bool,
    threads: usize,
    ro: ort::session::RunOptions,
}

static ENGINE_SLOT: Mutex<Option<Engine>> = Mutex::new(None);

//------------------------------------------------------------------
// 모델 파일이 다 있나
//=> 설정 extract.ocr 와 상관없이 파일만 본다. 없으면 OCR 을 끈 것처럼 동작한다(설계서 §10).
//
// -out: Ok(()) / Err(없는 파일 설명)
//------------------------------------------------------------------
pub fn models_present() -> Result<(), String> {
    let d = &cfg().ocr_dir;
    for f in [DET_FILE, REC_FILE, DICT_FILE] {
        let p = pyutil::join(d, f);
        if !std::path::Path::new(&p).is_file() {
            return Err(format!("OCR 모델 파일 없음: {}", p));
        }
    }
    Ok(())
}

//------------------------------------------------------------------
// OCR 을 쓸 수 있나 — 설정이 켜져 있고 모델 파일이 다 있을 때
//=> 모델이 없으면 로그에 한 번만 경고한다.
//------------------------------------------------------------------
pub fn enabled() -> bool {
    if !cfg().ocr {
        return false;
    }
    static WARNED: std::sync::Once = std::sync::Once::new();
    match models_present() {
        Ok(()) => true,
        Err(e) => {
            WARNED.call_once(|| crate::log::log(&format!("⚠️ {} — 스캔 문서 OCR 을 건너뜁니다", e)));
            false
        }
    }
}

// 그림 파일도 OCR 하나 — ocr 이 켜져 있고 ocr_images 도 켜져 있을 때
pub fn images_enabled() -> bool {
    cfg().ocr_images && enabled()
}

//------------------------------------------------------------------
// 엔진 적재 (이미 있으면 그대로)
//=> 1) 적재된 엔진의 아레나 · 스레드 설정이 지금 옵션과 다르면 내리고 다시 만든다(sdp_set_option 으로 바꾼 경우)
//   2) 아레나를 켜면 실행 옵션에 "실행마다 줄이기"(memory.enable_memory_arena_shrinkage = cpu:0)를 넣는다
//      — 켜기만 하면 쪽 크기가 매번 달라 창고가 늘기만 한다(2026-11-16 실측: 600MB → 2,070MB)
//
// -in: slot = 전역 엔진 자리
//
// -out: Ok(&mut Engine)
// -out: error = 모델 파일 없음 · 적재 실패면 Err("OCRError: …")
//------------------------------------------------------------------
fn ensure(slot: &mut Option<Engine>) -> Result<&mut Engine, String> {
    let (arena, th) = (cfg().ocr_arena, cfg().ocr_threads.max(1) as usize);
    // 옵션이 바뀌었으면 옛 엔진을 내린다 — 세션 설정은 만든 뒤 바꿀 수 없다
    if slot.as_ref().is_some_and(|e| e.arena != arena || e.threads != th) {
        *slot = None;
    }
    if slot.is_none() {
        models_present().map_err(|e| format!("OCRError: {}", e))?;
        let d = &cfg().ocr_dir;
        let det = crate::session::make_ocr_session(&pyutil::join(d, DET_FILE), th, arena)
            .map_err(|e| format!("OCRError: 검출 모델 적재 실패: {}", e))?;
        let rec = crate::session::make_ocr_session(&pyutil::join(d, REC_FILE), th, arena)
            .map_err(|e| format!("OCRError: 인식 모델 적재 실패: {}", e))?;
        let dict = rec::load_dict(&pyutil::join(d, DICT_FILE))?;
        let mut ro = ort::session::RunOptions::new().map_err(|e| format!("OCRError: 실행 옵션 만들기 실패: {}", e))?;
        // 아레나를 끈 세션에는 줄일 창고가 없으므로 빈 옵션(= 0.1.0 의 run 과 같음)
        if arena {
            ro.add_config_entry("memory.enable_memory_arena_shrinkage", "cpu:0")
                .map_err(|e| format!("OCRError: 실행 옵션 설정 실패: {}", e))?;
        }
        *slot = Some(Engine { det, rec, dict, arena, threads: th, ro });
    }
    Ok(slot.as_mut().unwrap())
}

//------------------------------------------------------------------
// 엔진 내리기 — 대기열을 다 처리했거나 멈출 때 메모리를 돌려준다
//------------------------------------------------------------------
pub fn unload() {
    *ENGINE_SLOT.lock().unwrap_or_else(|p| p.into_inner()) = None;
}

//------------------------------------------------------------------
// 그림 한 장 OCR (핵심)
//=> 1) 긴 변이 4000 을 넘으면 줄이고, 짧은 변이 30 미만이면 늘린다(좌표는 나중에 되돌린다)
//   2) 검출
//   3) 쪽 방향 판별(ocr_orient) — 누운 · 뒤집힌 쪽이면 그림과 상자를 함께 돌려 바로 세운다
//      (검출은 다시 하지 않는다. 0° 면 아무것도 바꾸지 않아 지금과 결과가 같다)
//   4) 상자마다 바로 세워 잘라내기 → 인식
//   5) 빈 글·점수 0.5 미만 줄은 버린다
//
// -in: img = BGR 그림
//
// -out: Ok((줄 목록 — 읽는 순서, 바로 세운 그림 좌표, 시계 방향으로 돌린 각도 0 · 90 · 180 · 270))
//       각도가 90 · 270 이면 좌표의 가로 · 세로 범위가 원래 그림과 바뀐다(가로 = img.h, 세로 = img.w)
// -out: error = 모델 적재·추론 실패면 Err("OCRError: …")
//------------------------------------------------------------------
pub fn ocr_image(img: &Bgr) -> Result<(Vec<Line>, u16), String> {
    if img.w == 0 || img.h == 0 {
        return Ok((vec![], 0));
    }
    // 1) 크기 한계 — 비율을 기억해 두었다가 상자 좌표를 원래 크기로 되돌린다
    let mut work = std::borrow::Cow::Borrowed(img);
    let mut scale = 1f32;
    if img.w.max(img.h) > MAX_SIDE {
        scale = MAX_SIDE as f32 / img.w.max(img.h) as f32;
    } else if img.w.min(img.h) < MIN_SIDE {
        scale = MIN_SIDE as f32 / img.w.min(img.h) as f32;
    }
    if scale != 1.0 {
        let nw = ((img.w as f32 * scale) as usize).max(1);
        let nh = ((img.h as f32 * scale) as usize).max(1);
        work = std::borrow::Cow::Owned(imgops::resize_linear(img, nw, nh));
    }
    let mut g = ENGINE_SLOT.lock().unwrap_or_else(|p| p.into_inner());
    let eng = ensure(&mut g)?;
    // 2) 검출
    let mut quads = det::detect(&mut eng.det, &work, &eng.ro)?;
    // 3) 쪽 방향 — 돌려야 하면 그림을 돌리고 상자도 같은 식으로 옮긴다(검출 결과는 방향과 거의 무관)
    let rot = if cfg().ocr_orient {
        orient::decide(&mut eng.rec, &eng.dict, &eng.ro, &work, &quads, cfg().ocr_orient_flip)?
    } else {
        0
    };
    if rot != 0 {
        let (w0, h0) = (work.w, work.h);
        let moved: Vec<[[f32; 2]; 4]> = quads.iter()
            // 꼭짓점을 옮긴 뒤 다시 [왼위, 오른위, 오른아래, 왼아래] 순서로 — rotate_crop 이 이 순서로 가로 · 세로를 잰다
            .map(|q| geom::order_clockwise(&q.map(|p| imgops::rot_point(p, rot, w0, h0))))
            .collect();
        // 읽는 순서도 바로 선 쪽 기준으로 다시 정한다(검출과 같은 규칙)
        quads = det::sort_boxes(moved);
        work = std::borrow::Cow::Owned(imgops::rotate(&work, rot));
    }
    // 4) 잘라내기 → 인식
    let mut kept = vec![];
    let mut crops = vec![];
    for q in quads {
        if let Some(c) = imgops::rotate_crop(&work, &q) {
            kept.push(q);
            crops.push(c);
        }
    }
    let texts = rec::recognize(&mut eng.rec, &eng.dict, &crops, &eng.ro)?;
    drop(g);
    // 5) 걸러 내고 좌표를 원래 크기로(돌렸으면 돌린 그림 기준 — 배율은 가로 · 세로 같으므로 그대로 나눈다)
    let mut out = vec![];
    for (q, (t, s)) in kept.into_iter().zip(texts) {
        if pyutil::strip(&t).is_empty() || s < TEXT_SCORE {
            continue;
        }
        let quad = q.map(|p| [p[0] / scale, p[1] / scale]);
        out.push(Line { text: t, score: s, quad });
    }
    Ok((out, rot))
}

// 돌린 각도가 90 · 270 이면 가로 · 세로가 바뀐다 — 쪽 크기를 바로 선 쪽 기준으로 맞출 때 쓴다
pub fn swaps_sides(rot: u16) -> bool {
    rot == 90 || rot == 270
}

//------------------------------------------------------------------
// 상한 배율 — 그림(또는 200dpi 로 그릴 쪽)이 A4 보다 크면 얼마나 줄일지
//=> 긴 변 · 짧은 변을 따로 본다(가로 쪽도 같은 규칙). A4 이하는 1.0 = 지금과 똑같다.
//    배율 = min(1, 2339 / 긴 변, 1654 / 짧은 변)
//    배율이 0.95 이상(A4 보다 5% 이내로 큼)이면 1.0 — 거의 A4 인 쪽은 그대로 둔다(CAP_MIN_SCALE)
//
// -in: w · h = 가로 · 세로 픽셀(200dpi 기준 또는 그림 파일 원래 크기)
//
// -out: 배율(0 초과 1 이하)
// -out: error = 없음 (0 크기면 1.0)
//------------------------------------------------------------------
pub fn cap_scale(w: f32, h: f32) -> f32 {
    let (long, short) = (w.max(h), w.min(h));
    if short <= 0.0 {
        return 1.0;
    }
    let s = (CAP_LONG / long).min(CAP_SHORT / short);
    // 조금 큰 쪽은 줄여 봐야 속도 · 메모리 이득이 없고 결과만 달라진다
    if s >= CAP_MIN_SCALE { 1.0 } else { s }
}

//------------------------------------------------------------------
// 작은 쪽 확대 배율 — 200dpi 로 그릴 쪽이 A4 보다 작으면 얼마나 키울지 (쪽방향보정 설계서 5장)
//=> cap_scale 의 반대. 긴 변 · 짧은 변을 따로 보고 두 변 모두 A4 를 넘지 않는 데까지, 최대 UP_MAX 배.
//    배율 = min(UP_MAX, 2339 / 긴 변, 1654 / 짧은 변)
//    배율이 UP_MIN 이하(A4 보다 5% 안쪽으로 작음 — Letter 등)이면 1.0 — 거의 A4 인 쪽은 그대로 둔다
//
// -in: w · h = 가로 · 세로 픽셀(200dpi 기준)
//
// -out: 배율(1 이상 UP_MAX 이하)
// -out: error = 없음 (0 크기면 1.0)
//------------------------------------------------------------------
pub fn up_scale(w: f32, h: f32) -> f32 {
    let (long, short) = (w.max(h), w.min(h));
    if short <= 0.0 {
        return 1.0;
    }
    let s = (CAP_LONG / long).min(CAP_SHORT / short).min(UP_MAX);
    // 조금 작은 쪽은 키워 봐야 글자가 거의 늘지 않고 결과만 달라진다
    if s <= UP_MIN { 1.0 } else { s }
}

//------------------------------------------------------------------
// OCR 줄 좌표를 k 배 하기 — 줄인 그림에서 찾은 상자를 원래 기준 좌표로 되돌린다
//
// -in: lines = OCR 줄 / k = 곱할 배율(1.0 이면 그대로)
//
// -out: 좌표를 바꾼 줄
// -out: error = 없음
//------------------------------------------------------------------
pub fn scale_lines(mut lines: Vec<Line>, k: f32) -> Vec<Line> {
    if k != 1.0 {
        for l in &mut lines {
            l.quad = l.quad.map(|p| [p[0] * k, p[1] * k]);
        }
    }
    lines
}

//------------------------------------------------------------------
// 그림 파일 한 장 OCR — 큰 그림은 A4 크기로 줄여서 (OCR 속도개선 설계서 4-1 · 4-2)
//=> PDF 와 달리 다시 그릴 수 없으므로 픽셀을 줄인다.
//    1) 상한이 꺼져 있거나(ocr_page_cap · ocr_image_cap) A4 이하면 지금처럼 ocr_image 그대로
//    2) 넓이 평균(resize_area)으로 줄인다 — 많이 줄여도 가는 획이 사라지지 않게
//    3) 검출 · 인식을 모두 줄인 그림으로 한다(검출만 줄이면 실측 글자 F1 0.81 로 떨어짐)
//    4) 상자 좌표는 원래 그림 픽셀로 되돌린다(그림 파일 캐시는 원래 픽셀 좌표, dpi 0)
//
// -in: img = 그림 파일에서 읽은 쪽 그림(원래 크기)
//
// -out: Ok((줄 목록 — 원래 크기 · 바로 세운 그림 좌표, 시계 방향으로 돌린 각도))
// -out: error = 추론 실패면 Err("OCRError: …")
//------------------------------------------------------------------
pub fn ocr_image_file(img: &Bgr) -> Result<(Vec<Line>, u16), String> {
    let s = if cfg().ocr_page_cap && cfg().ocr_image_cap { cap_scale(img.w as f32, img.h as f32) } else { 1.0 };
    if s >= 1.0 {
        return ocr_image(img);
    }
    let nw = ((img.w as f32 * s).round() as usize).max(1);
    let nh = ((img.h as f32 * s).round() as usize).max(1);
    let small = imgops::resize_area(img, nw, nh);
    // 실제로 줄어든 비율로 되돌린다(반올림 오차를 줄이려고 가로 · 세로 평균)
    let back = (img.w as f32 / nw as f32 + img.h as f32 / nh as f32) / 2.0;
    let (lines, rot) = ocr_image(&small)?;
    Ok((scale_lines(lines, back), rot))
}

//------------------------------------------------------------------
// 줄 묶기 — OCR 상자들 → 화면의 한 줄씩 (설계서 §5-4)
//=> 1) 상자를 세로 가운데로 정렬하고, 세로로 절반 이상 겹치면 같은 줄로 묶는다
//   2) 줄 안에서는 왼쪽부터 잇고, 상자 사이 틈이 글자 높이의 0.5배보다 넓으면 띄어쓰기를 넣는다
//      (한국어 인식은 상자 안 띄어쓰기를 자주 빠뜨린다 — 상자 사이 띄어쓰기라도 살린다)
//
// -in: lines = OCR 줄 목록 (bbox 를 쓴다)
//
// -out: [(줄 글, [x0, y0, x1, y1], 평균 높이)] — 위에서 아래 순서
// -out: error = 없음
//------------------------------------------------------------------
pub fn group_rows(lines: &[(String, [f32; 4])]) -> Vec<(String, [f32; 4], f32)> {
    group_rows_ids(lines).into_iter().map(|(t, b, h, _)| (t, b, h)).collect()
}

//------------------------------------------------------------------
// 줄 묶기 + 줄마다 묶인 상자 번호 (group_rows 본체)
//=> group_rows 와 같은 규칙으로 묶고, 줄마다 어느 상자들이 들어갔는지(왼쪽부터)도 함께 돌려준다.
//   구조 분석(layout)이 "상자 사이 틈이 넓은 표 행"을 가려낼 때 쓴다.
//
// -in: lines = OCR 줄 목록 (bbox 를 쓴다)
//
// -out: [(줄 글, [x0, y0, x1, y1], 평균 높이, 상자 번호들)] — 위에서 아래 순서
// -out: error = 없음
//------------------------------------------------------------------
pub fn group_rows_ids(lines: &[(String, [f32; 4])]) -> Vec<(String, [f32; 4], f32, Vec<usize>)> {
    let mut idx: Vec<usize> = (0..lines.len()).collect();
    idx.sort_by(|&a, &b| {
        let ca = (lines[a].1[1] + lines[a].1[3]) / 2.0;
        let cb = (lines[b].1[1] + lines[b].1[3]) / 2.0;
        ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)
    });
    // 줄마다 (상자 번호들, 세로 범위)
    let mut rows: Vec<(Vec<usize>, f32, f32)> = vec![];
    for i in idx {
        let b = lines[i].1;
        let h = (b[3] - b[1]).max(1.0);
        let mut placed = false;
        if let Some(r) = rows.last_mut() {
            let rh = (r.2 - r.1).max(1.0);
            let ov = b[3].min(r.2) - b[1].max(r.1);
            if ov >= 0.5 * h.min(rh) {
                r.0.push(i);
                r.1 = r.1.min(b[1]);
                r.2 = r.2.max(b[3]);
                placed = true;
            }
        }
        if !placed {
            rows.push((vec![i], b[1], b[3]));
        }
    }
    let mut out = vec![];
    for (mut ids, _, _) in rows {
        ids.sort_by(|&a, &b| lines[a].1[0].partial_cmp(&lines[b].1[0]).unwrap_or(std::cmp::Ordering::Equal));
        let avg_h = ids.iter().map(|&i| lines[i].1[3] - lines[i].1[1]).sum::<f32>() / ids.len() as f32;
        let mut text = String::new();
        let mut bb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        let mut prev_x1: Option<f32> = None;
        for &i in &ids {
            let (t, b) = (&lines[i].0, lines[i].1);
            if let Some(px) = prev_x1 {
                if b[0] - px > 0.5 * avg_h {
                    text.push(' ');
                }
            }
            text.push_str(pyutil::strip(t));
            prev_x1 = Some(prev_x1.map_or(b[2], |p| p.max(b[2])));
            bb = [bb[0].min(b[0]), bb[1].min(b[1]), bb[2].max(b[2]), bb[3].max(b[3])];
        }
        out.push((text, bb, avg_h, ids));
    }
    out
}

// ── 어느 쪽을 OCR 하나 (설계서 §4-2) ───────────────────────────

// 그림 넓이를 셀 격자 — 겹친 그림을 한 번만 센다(2026-11-13 분포 실측과 같은 40×40)
const GRID: usize = 40;

// 그림 상자들이 쪽을 덮는 비율(0~1)
fn coverage(boxes: &[[f32; 4]], pw: f32, ph: f32) -> f32 {
    if boxes.is_empty() || pw <= 0.0 || ph <= 0.0 {
        return 0.0;
    }
    let mut m = [[false; GRID]; GRID];
    for b in boxes {
        let x0 = ((b[0].max(0.0) / pw) * GRID as f32) as usize;
        let x1 = (((b[2].min(pw) / pw) * GRID as f32).ceil() as usize).min(GRID);
        let y0 = ((b[1].max(0.0) / ph) * GRID as f32) as usize;
        let y1 = (((b[3].min(ph) / ph) * GRID as f32).ceil() as usize).min(GRID);
        for row in m.iter_mut().take(y1).skip(y0) {
            for c in row.iter_mut().take(x1).skip(x0) {
                *c = true;
            }
        }
    }
    m.iter().flatten().filter(|&&c| c).count() as f32 / (GRID * GRID) as f32
}

//------------------------------------------------------------------
// PDF 쪽 가운데 OCR 할 쪽 고르기 (설계서 §4-2)
//=> 1) 반복 그림 — 문서 쪽의 절반 이상(3쪽 이상)에 같은 자리·같은 픽셀 크기로 나오는 그림(로고·배경·틀)
//   2) 쪽마다: 공백 뺀 글자 수 n, 모든 그림 넓이 all, 반복 아니고 쪽의 2% 이상인 그림 넓이 real
//      · n < 10 이고 all ≥ 30%                → OCR (스캔 쪽)
//      · n < 10                               → 건너뜀 (빈 쪽·간지)
//      · real ≥ 70% 이고 n < 200              → OCR (스캔 쪽에 쪽 번호·도장 글자가 얹힌 쪽, 그림 속 글 디자인)
//      · 그 밖                                → 글자층만
//   3) 문서 전체에 글자층 글이 거의 없는데(공백 뺀 20자 미만) 고른 쪽이 없으면 모든 쪽을 고른다 —
//      글을 그림 개체가 아니라 선 · 도형(벡터)으로 그린 PDF 는 1) · 2) 로는 한 쪽도 안 잡혀 '글자 없는 PDF' 로
//      실패하고, 예전 실패 다시 보기와 맞물려 순찰마다 실패 알림이 떴다(2026-11-13, 최저임금 전단지 등 2건)
//   4) 앞에서부터 ocr_max_pages 쪽까지만
//
// -in: pages = pdfium 으로 읽은 쪽들(images · page_w · page_h 가 채워져 있어야 한다)
//
// -out: (OCR 할 쪽 번호(0부터) 목록, 상한에 걸려 뺀 쪽 수)
// -out: error = 없음
//------------------------------------------------------------------
pub fn pdf_ocr_pages(pages: &[crate::pdfium::PageText]) -> (Vec<usize>, usize) {
    let np = pages.len();
    // 1) 반복 그림 열쇠 — (정수로 반올림한 범위, 픽셀 크기)
    let key = |o: &crate::pdfium::ImgObj| ([o.b[0].round() as i32, o.b[1].round() as i32, o.b[2].round() as i32, o.b[3].round() as i32], o.px);
    let mut rep: std::collections::HashMap<([i32; 4], (u32, u32)), usize> = std::collections::HashMap::new();
    for p in pages {
        let set: std::collections::HashSet<_> = p.images.iter().map(key).collect();
        for k in set {
            *rep.entry(k).or_insert(0) += 1;
        }
    }
    let need = (np as f32 * 0.5).max(3.0);
    let repeated = |o: &crate::pdfium::ImgObj| np >= 3 && rep.get(&key(o)).map_or(false, |&c| c as f32 >= need);
    let mut out = vec![];
    for (i, p) in pages.iter().enumerate() {
        if p.images.is_empty() {
            continue;
        }
        let n = p.text.chars().filter(|c| !c.is_whitespace()).count();
        let all: Vec<[f32; 4]> = p.images.iter().map(|o| o.b).collect();
        let page_area = p.page_w * p.page_h;
        let real: Vec<[f32; 4]> = p.images.iter()
            .filter(|o| !repeated(o) && (o.b[2] - o.b[0]) * (o.b[3] - o.b[1]) >= 0.02 * page_area)
            .map(|o| o.b).collect();
        // 그림 좌표는 PDF 좌표(아래가 0) — 넓이 비율만 보므로 위아래 방향은 상관없다
        let cov_all = coverage(&all, p.page_w, p.page_h);
        let cov_real = coverage(&real, p.page_w, p.page_h);
        let hit = if n < 10 { cov_all >= 0.3 } else { cov_real >= 0.7 && n < 200 };
        if hit {
            out.push(i);
        }
    }
    // 3) 글자층이 사실상 없는 문서 — 그림 개체가 없어도(벡터로 그린 글) 모든 쪽을 OCR 한다
    let total: usize = pages.iter().map(|p| p.text.chars().filter(|c| !c.is_whitespace()).count()).sum();
    if out.is_empty() && total < 20 {
        out = (0..np).collect();
    }
    let cap = cfg().ocr_max_pages.max(1) as usize;
    let dropped = out.len().saturating_sub(cap);
    out.truncate(cap);
    (out, dropped)
}

//------------------------------------------------------------------
// 글자층과 겹친 OCR 줄 빼기 (설계서 §4-2 ②)
//=> OCR 줄 상자 넓이의 절반 이상이 글자층 글자 상자들과 겹치면 버린다 — 그 자리에는 더 정확한
//   글자층 글이 이미 있다(쪽 번호·도장 글자를 두 번 넣지 않게).
//
// -in: lines = OCR 줄 / chars = 글자층 글자 상자 [x0, y0, x1, y1] (같은 그림 좌표)
//
// -out: 남긴 줄
// -out: error = 없음
//------------------------------------------------------------------
pub fn drop_overlapping(lines: Vec<Line>, chars: &[[f32; 4]]) -> Vec<Line> {
    if chars.is_empty() {
        return lines;
    }
    lines.into_iter().filter(|l| {
        let b = l.bbox();
        let area = ((b[2] - b[0]) * (b[3] - b[1])).max(1.0);
        // 글자 상자끼리는 거의 겹치지 않으므로 겹친 넓이를 더해 근사한다
        let ov: f32 = chars.iter().map(|c| {
            let w = (b[2].min(c[2]) - b[0].max(c[0])).max(0.0);
            let h = (b[3].min(c[3]) - b[1].max(c[1])).max(0.0);
            w * h
        }).sum();
        ov < 0.5 * area
    }).collect()
}

// ── 추출 → 색인 사이의 OCR 정보 ─────────────────────────────

//------------------------------------------------------------------
// 문서 하나의 OCR 상태 (추출이 남기고 sync_doc 이 읽는다)
// -필드: pending = 아직 OCR 하지 않은 쪽(1부터, 그림 파일은 [1]) — 비어 있지 않으면 대기열로 간다
// -필드: done    = OCR 글이 들어간 쪽(1부터) — 그 쪽 청크에 ocr 표시를 단다
// -필드: whole   = 그림 파일처럼 모든 청크가 OCR 글인가
//------------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct DocOcr {
    pub pending: Vec<usize>,
    pub done: Vec<usize>,
    pub whole: bool,
}

// 추출은 준비 스레드에서도 돌므로 스레드 사이에서 보이는 표에 남긴다(열쇠 = 절대 경로)
static DOC_OCR: Mutex<Option<std::collections::HashMap<String, DocOcr>>> = Mutex::new(None);

// 추출 결과의 OCR 상태 남기기 — 비어 있으면 지운다
pub fn note(path: &str, info: DocOcr) {
    let mut g = DOC_OCR.lock().unwrap_or_else(|p| p.into_inner());
    let m = g.get_or_insert_with(Default::default);
    let k = pyutil::abspath(path);
    if info.pending.is_empty() && info.done.is_empty() && !info.whole {
        m.remove(&k);
    } else {
        m.insert(k, info);
    }
}

// 남긴 OCR 상태 꺼내기(꺼내면 지운다)
pub fn take_note(path: &str) -> DocOcr {
    let mut g = DOC_OCR.lock().unwrap_or_else(|p| p.into_inner());
    g.get_or_insert_with(Default::default).remove(&pyutil::abspath(path)).unwrap_or_default()
}

// ── 캐시 ─────────────────────────────────────────────────────

//------------------------------------------------------------------
// 캐시의 한 줄
// -필드: t = 글 / s = 점수 / b = [x0, y0, x1, y1] (200dpi 그림 픽셀 좌표)
//------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedLine {
    pub t: String,
    pub s: f32,
    pub b: [f32; 4],
}

//------------------------------------------------------------------
// 파일 하나의 OCR 캐시 (설계서 §7)
// -필드: engine = 만든 엔진 이름(ENGINE 과 다르면 버림)
// -필드: dpi    = PDF 쪽을 그린 해상도(그림 파일은 0)
// -필드: pages  = 쪽 번호(1부터) → 줄 목록. OCR 해 본 쪽은 줄이 없어도 빈 목록으로 남는다
// -필드: rot    = 쪽 번호 → 바로 세우려고 시계 방향으로 돌린 각도(90 · 180 · 270). 돌리지 않은 쪽은 넣지 않는다
//                (그래서 바로 선 문서의 캐시 파일은 이전과 글자 하나까지 같고, 옛 캐시는 모두 0 으로 읽힌다)
//                90 · 270 인 쪽의 줄 좌표는 가로 · 세로가 바뀐 바로 선 쪽 기준이다
//------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Cache {
    pub engine: String,
    pub dpi: f32,
    pub pages: BTreeMap<usize, Vec<CachedLine>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rot: BTreeMap<usize, u16>,
}

impl Cache {
    //------------------------------------------------------------------
    // 쪽 하나의 결과 넣기 — 줄 목록과 돌린 각도를 함께 적는다
    //=> 같은 쪽을 다시 넣을 때 옛 각도가 남지 않게, 0 이면 각도 표에서 지운다.
    //
    // -in: no = 쪽 번호(1부터) / lines = 캐시 줄 / rot = 돌린 각도
    //
    // -out: 없음
    // -out: error = 없음
    //------------------------------------------------------------------
    pub fn put_page(&mut self, no: usize, lines: Vec<CachedLine>, rot: u16) {
        self.pages.insert(no, lines);
        if rot == 0 {
            self.rot.remove(&no);
        } else {
            self.rot.insert(no, rot);
        }
    }

    // 쪽 하나의 돌린 각도(없으면 0)
    pub fn page_rot(&self, no: usize) -> u16 {
        self.rot.get(&no).copied().unwrap_or(0)
    }
}

// 캐시 폴더를 주지 않았을 때 쓰는 프로세스 메모리 캐시(해시 → 캐시)
static MEM_CACHE: Mutex<Option<std::collections::HashMap<String, Cache>>> = Mutex::new(None);

//------------------------------------------------------------------
// 캐시 파일 경로 — <캐시 폴더>\<해시 앞 2자>\<해시>.json
//=> SimpleRAG 의 <데이터 폴더>\ocr_cache 와 같은 모양이라 호스트가 그 폴더를 넘기면 옛 캐시를 그대로 쓴다.
//
// -in: hash = 파일 내용 SHA-1
//
// -out: Some(경로) / None = 캐시 폴더 없음(메모리 캐시를 쓴다)
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn cache_path(hash: &str) -> Option<String> {
    let root = cfg().ocr_cache_dir.clone()?;
    let dir = pyutil::join(&root, &hash[..hash.len().min(2)]);
    Some(pyutil::join(&dir, &format!("{}.json", hash)))
}

//------------------------------------------------------------------
// 캐시 읽기 — 없거나, 깨졌거나, 엔진이 다르면 None
//------------------------------------------------------------------
pub fn load_cache(hash: &str) -> Option<Cache> {
    let Some(path) = cache_path(hash) else {
        // 캐시 폴더가 없으면 메모리에서
        let g = MEM_CACHE.lock().unwrap_or_else(|p| p.into_inner());
        return g.as_ref().and_then(|m| m.get(hash)).filter(|c| c.engine == ENGINE).cloned();
    };
    let s = std::fs::read_to_string(path).ok()?;
    let c: Cache = serde_json::from_str(&s).ok()?;
    if c.engine != ENGINE {
        return None;
    }
    Some(c)
}

//------------------------------------------------------------------
// 캐시 쓰기 — 임시 파일에 쓴 뒤 바꿔 넣는다(쓰다 꺼져도 반쪽 파일이 남지 않게)
//
// -out: error = 폴더·파일 쓰기 실패면 Err(오류문)
//------------------------------------------------------------------
pub fn save_cache(hash: &str, c: &Cache) -> Result<(), String> {
    let Some(p) = cache_path(hash) else {
        // 캐시 폴더가 없으면 메모리에 둔다(프로세스가 끝나면 사라짐)
        MEM_CACHE.lock().unwrap_or_else(|p| p.into_inner()).get_or_insert_with(Default::default).insert(hash.to_string(), c.clone());
        return Ok(());
    };
    std::fs::create_dir_all(pyutil::dirname(&p)).map_err(|e| format!("OCR 캐시 폴더 만들기 실패: {}", e))?;
    let tmp = format!("{}.tmp", p);
    std::fs::write(&tmp, serde_json::to_string(c).unwrap_or_default()).map_err(|e| format!("OCR 캐시 쓰기 실패: {}", e))?;
    std::fs::rename(&tmp, &p).map_err(|e| format!("OCR 캐시 바꿔 넣기 실패: {}", e))
}

// 캐시 지우기 — 없으면 조용히 넘어간다
pub fn remove_cache(hash: &str) {
    match cache_path(hash) {
        Some(p) => {
            let _ = std::fs::remove_file(p);
        }
        None => {
            if let Some(m) = MEM_CACHE.lock().unwrap_or_else(|p| p.into_inner()).as_mut() {
                m.remove(hash);
            }
        }
    }
}

// OCR 줄 → 캐시 줄 (점수는 소수 3자리)
pub fn to_cached(lines: &[Line]) -> Vec<CachedLine> {
    lines.iter().map(|l| CachedLine { t: l.text.clone(), s: (l.score * 1000.0).round() / 1000.0, b: l.bbox() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 상한 배율 — A4 · Letter · 1픽셀 넘는 A4 는 그대로, 1.33배 쪽은 A4 로
    #[test]
    fn cap_scale_rules() {
        assert_eq!(cap_scale(1654.0, 2339.0), 1.0);
        assert_eq!(cap_scale(1655.0, 2339.0), 1.0);
        assert_eq!(cap_scale(1700.0, 2200.0), 1.0);
        assert_eq!(cap_scale(2339.0, 1654.0), 1.0);
        let s = cap_scale(2199.0, 3110.0);
        assert!((s - 2339.0 / 3110.0).abs() < 1e-4);
        // 4K 화면 캡처 — 짧은 변보다 긴 변이 먼저 걸린다
        assert!((cap_scale(3840.0, 2160.0) - 2339.0 / 3840.0).abs() < 1e-4);
    }

    // 작은 쪽 확대 — A4 · Letter · 16:9 슬라이드는 그대로, A5 · A6 은 1.4배, A4 에 조금 모자란 쪽은 A4 까지만
    #[test]
    fn up_scale_rules() {
        assert_eq!(up_scale(1654.0, 2339.0), 1.0);
        assert_eq!(up_scale(1700.0, 2200.0), 1.0);
        assert_eq!(up_scale(2667.0, 1500.0), 1.0);
        assert_eq!(up_scale(2339.0 * 1.2, 1654.0 * 1.2), 1.0);
        assert!((up_scale(827.0, 1170.0) - 1.4).abs() < 1e-6);
        assert!((up_scale(1166.0, 1654.0) - 1.4).abs() < 1e-6);
        // A4 의 80% 크기 → 1.25 배(A4 에 닿는 데까지)
        let s = up_scale(1654.0 * 0.8, 2339.0 * 0.8);
        assert!((s - 1.25).abs() < 1e-3);
        assert_eq!(up_scale(0.0, 100.0), 1.0);
    }

    #[test]
    fn rows_join_with_gap_space() {
        // 같은 줄 두 상자(틈 넓음) + 아래 줄 하나
        let lines = vec![
            ("수신:".to_string(), [0.0, 0.0, 40.0, 20.0]),
            ("나우로보틱스".to_string(), [60.0, 1.0, 160.0, 21.0]),
            ("참조".to_string(), [0.0, 40.0, 30.0, 60.0]),
        ];
        let r = group_rows(&lines);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].0, "수신: 나우로보틱스");
        assert_eq!(r[1].0, "참조");
    }

    #[test]
    fn rows_no_space_when_touching() {
        let lines = vec![("가나".to_string(), [0.0, 0.0, 40.0, 20.0]), ("다".to_string(), [42.0, 0.0, 60.0, 20.0])];
        assert_eq!(group_rows(&lines)[0].0, "가나다");
    }
}
