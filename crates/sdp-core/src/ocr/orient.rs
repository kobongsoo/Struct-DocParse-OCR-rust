//------------------------------------------------------------------
// 쪽 방향 판별 (OCR 쪽방향보정 설계서 2026-11-17, 방식 A)
//=> 누운 · 뒤집힌 채 스캔된 쪽을 바로 세우려면 몇 도 돌려야 하는지 정한다. 새 모델 없이
//   이미 있는 검출 상자 모양과 인식 점수만 쓴다.
//    1) 축 판정 — 줄 모양 상자(긴 변 ≥ 짧은 변×2)가 대부분 세로로 길면 누운 쪽, 가로로 길면 선 쪽
//    2) 방향 판정 — 긴 상자 몇 개를 "지금 방향"과 "180° 뒤집은 방향"으로 인식해 점수가 높은 쪽
//    3) 애매하면 지금과 같은 결과가 나오는 쪽(선 쪽 0°, 누운 쪽 270°)을 고른다
//   돌리는 각도는 늘 "시계 방향으로 몇 도"(0 · 90 · 180 · 270) — imgops::rotate 와 같은 뜻이다.
//------------------------------------------------------------------

use ort::session::Session;

use super::imgops::{self, Bgr};
use super::rec;

type P = [f32; 2];

// 줄 모양 상자 — 긴 변이 짧은 변의 이 배 이상(글자 한두 개짜리 정사각형 상자는 방향을 알려 주지 않는다)
const LINE_RATIO: f32 = 2.0;
// 줄 모양 상자가 이보다 적으면 판단하지 않는다(표지 · 그림 쪽)
const MIN_LINES: usize = 3;
// 세로 상자 비율이 이 이상이면 누운 쪽, LOW 이하면 선 쪽, 그 사이는 섞인 쪽(보정 안 함)
const TALL_HIGH: f32 = 0.6;
const TALL_LOW: f32 = 0.4;
// 축 판정에 쓰는 상자 — 긴 변이 가로 · 세로축에서 이 각도(도) 안쪽인 것만(부채꼴로 비스듬히 놓인 글은 쪽 방향을 알려 주지 않는다)
const AXIS_TOL_DEG: f32 = 15.0;
// 방향 판정에 쓸 표본 상자 수(긴 것부터)
const SAMPLES: usize = 6;
// 표본 그림은 앞쪽만 잘라 쓴다 — 가로가 세로의 이 배를 넘으면 자른다(인식 시간은 그림 가로에 비례. 글자 몇 개면 방향을 안다)
const SAMPLE_MAX_RATIO: usize = 6;
// 뒤집은 방향이 이만큼 이상 점수가 높고, 그 점수가 FLIP_MIN 이상이어야 뒤집는다
const FLIP_MARGIN: f32 = 0.10;
const FLIP_MIN: f32 = 0.6;

//------------------------------------------------------------------
// 상자 하나의 가로 · 세로 길이 (rotate_crop 과 같은 계산)
//
// -in: q = [왼위, 오른위, 오른아래, 왼아래]
//
// -out: (가로, 세로)
// -out: error = 없음
//------------------------------------------------------------------
fn sides(q: &[P; 4]) -> (f32, f32) {
    let d = |a: P, b: P| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    (d(q[0], q[1]).max(d(q[2], q[3])), d(q[0], q[3]).max(d(q[1], q[2])))
}

//------------------------------------------------------------------
// 상자의 긴 변이 가로축 · 세로축 가까이에 놓였나
//=> 긴 변 방향의 각도를 0~90° 로 접어, 0° 또는 90° 에서 AXIS_TOL_DEG 안이면 true.
//   비스듬한 글(부채꼴 배치 · 기울인 도장)은 쪽이 누웠는지와 상관이 없어 빼야 한다.
//
// -in: q = [왼위, 오른위, 오른아래, 왼아래] / tall = 세로로 긴 상자인가
//
// -out: true = 축에 맞춰 놓인 상자
// -out: error = 없음
//------------------------------------------------------------------
fn on_axis(q: &[P; 4], tall: bool) -> bool {
    // 긴 변 — 세로로 길면 왼위→왼아래, 아니면 왼위→오른위
    let (a, b) = if tall { (q[0], q[3]) } else { (q[0], q[1]) };
    let deg = (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees().abs() % 180.0;
    // 0~90 으로 접는다(180 근처도 가로)
    let deg = if deg > 90.0 { 180.0 - deg } else { deg };
    deg <= AXIS_TOL_DEG || deg >= 90.0 - AXIS_TOL_DEG
}

//------------------------------------------------------------------
// 축 판정 — 이 쪽이 누웠나, 섰나
//=> 1) 줄 모양이고 축에 맞춰 놓인 상자만 골라 세로로 긴 것 / 가로로 긴 것을 센다
//   2) 세로 비율 ≥ 0.6 → 누운 쪽, ≤ 0.4 → 선 쪽, 사이 · 상자 부족 → 모름
//   3) 다수 쪽 상자를 긴 것부터 표본으로 돌려준다(방향 판정용)
//
// -in: quads = 검출 상자들
//
// -out: Some((누웠나, 표본 상자 번호 — 긴 것부터 최대 SAMPLES 개)) / None = 판단 안 함
// -out: error = 없음
//------------------------------------------------------------------
pub fn axis(quads: &[[P; 4]]) -> Option<(bool, Vec<usize>)> {
    // (번호, 세로로 긴가, 긴 변)
    let lines: Vec<(usize, bool, f32)> = quads.iter().enumerate().filter_map(|(i, q)| {
        let (w, h) = sides(q);
        let (long, short) = (w.max(h), w.min(h));
        // 줄 모양만 — 정사각형에 가까운 상자는 어느 쪽으로도 읽힌다. 비스듬한 상자도 뺀다
        (short > 0.0 && long >= short * LINE_RATIO && on_axis(q, h > w)).then_some((i, h > w, long))
    }).collect();
    if lines.len() < MIN_LINES {
        return None;
    }
    let tall = lines.iter().filter(|l| l.1).count() as f32 / lines.len() as f32;
    let lying = if tall >= TALL_HIGH {
        true
    } else if tall <= TALL_LOW {
        false
    } else {
        // 누운 줄과 선 줄이 섞인 쪽(눕힌 표 한 칸 등) — 쪽 전체를 돌리면 다른 반이 망가진다
        return None;
    };
    let mut pick: Vec<&(usize, bool, f32)> = lines.iter().filter(|l| l.1 == lying).collect();
    // 긴 줄일수록 글자가 많아 점수가 믿을 만하다
    pick.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    Some((lying, pick.into_iter().take(SAMPLES).map(|l| l.0).collect()))
}

//------------------------------------------------------------------
// 그림의 왼쪽 max_w 픽셀만 남기기 (가로가 그보다 좁으면 그대로)
//
// -in: src = 그림 / max_w = 남길 가로 픽셀(1 이상으로 본다)
//
// -out: 잘라낸 그림(복사본)
// -out: error = 없음
//------------------------------------------------------------------
fn head(src: &Bgr, max_w: usize) -> Bgr {
    let nw = src.w.min(max_w.max(1));
    if nw == src.w {
        return src.clone();
    }
    let mut data = Vec::with_capacity(nw * src.h * 3);
    for y in 0..src.h {
        // 한 줄씩 앞쪽 nw 픽셀만 옮긴다
        let s = y * src.w * 3;
        data.extend_from_slice(&src.data[s..s + nw * 3]);
    }
    Bgr { w: nw, h: src.h, data }
}

// 값 목록의 중앙값(빈 목록이면 0)
fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

//------------------------------------------------------------------
// 뒤집을지 — 두 방향 점수 중앙값 비교
//=> 뒤집은 쪽이 FLIP_MARGIN 이상 높고 FLIP_MIN 이상일 때만 뒤집는다(애매하면 지금 방향).
//
// -in: keep = 지금 방향 점수들 / flip = 180° 뒤집은 방향 점수들(같은 상자 순서)
//
// -out: true = 뒤집는다
// -out: error = 없음
//------------------------------------------------------------------
pub fn should_flip(keep: &[f32], flip: &[f32]) -> bool {
    let (mk, mf) = (median(keep.to_vec()), median(flip.to_vec()));
    mf - mk >= FLIP_MARGIN && mf >= FLIP_MIN
}

//------------------------------------------------------------------
// 쪽 방향 정하기 (핵심)
//=> 1) axis 로 누웠는지 본다 — 모르면 0(보정 안 함)
//   2) 선 쪽인데 flip(뒤집힘 확인)이 꺼져 있으면 0 — 인식을 더 하지 않는다
//   3) 표본 상자를 rotate_crop 으로 잘라(세로로 긴 상자는 이미 반시계 90° 돌아 나온다 = 270°)
//      그대로 · 180° 뒤집어 함께 인식하고, should_flip 으로 고른다
//   4) 기본 각도(선 쪽 0 · 누운 쪽 270) — 뒤집으면 +180
//
// -in: sess · dict · ro = 인식 모델(ocr::Engine 의 것)
// -in: img   = 검출에 쓴 그림
// -in: quads = 검출 상자들(img 좌표)
// -in: flip  = 선 쪽도 뒤집힘(180°)을 확인하나(ocr_orient_flip)
//
// -out: Ok(시계 방향 각도 0 · 90 · 180 · 270)
// -out: error = 인식 실패면 Err("OCRError: …")
//------------------------------------------------------------------
pub fn decide(sess: &mut Session, dict: &[String], ro: &ort::session::RunOptions, img: &Bgr, quads: &[[P; 4]], flip: bool) -> Result<u16, String> {
    let Some((lying, ids)) = axis(quads) else { return Ok(0) };
    if !lying && !flip {
        return Ok(0);
    }
    // 지금 규칙으로 자른 그림 + 그것을 뒤집은 그림을 한 번에 인식한다(같은 상자끼리 같은 크기라 함께 묶인다)
    let mut crops = vec![];
    for &i in &ids {
        if let Some(c) = imgops::rotate_crop(img, &quads[i]) {
            // 긴 줄은 앞쪽만 — 표본 인식 시간을 줄인다(뒤집은 그림은 같은 조각을 돌린 것이라 공정하다)
            let c = head(&c, c.h * SAMPLE_MAX_RATIO);
            let f = imgops::rotate(&c, 180);
            crops.push(c);
            crops.push(f);
        }
    }
    if crops.is_empty() {
        return Ok(if lying { 270 } else { 0 });
    }
    let res = rec::recognize(sess, dict, &crops, ro)?;
    let keep: Vec<f32> = res.iter().step_by(2).map(|r| r.1).collect();
    let flipped: Vec<f32> = res.iter().skip(1).step_by(2).map(|r| r.1).collect();
    let base: u16 = if lying { 270 } else { 0 };
    Ok(if should_flip(&keep, &flipped) { (base + 180) % 360 } else { base })
}

#[cfg(test)]
mod tests {
    use super::*;

    // 축에 맞춘 상자 [x0, y0] 부터 가로 w · 세로 h
    fn rect(x: f32, y: f32, w: f32, h: f32) -> [P; 4] {
        [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
    }

    #[test]
    fn upright_page_is_not_lying() {
        let q = vec![rect(0.0, 0.0, 200.0, 20.0), rect(0.0, 30.0, 150.0, 20.0), rect(0.0, 60.0, 300.0, 20.0), rect(0.0, 90.0, 20.0, 20.0)];
        let (lying, ids) = axis(&q).unwrap();
        assert!(!lying);
        // 정사각형 상자는 빠지고, 긴 것부터
        assert_eq!(ids, vec![2, 0, 1]);
    }

    #[test]
    fn sideways_page_is_lying() {
        let q = vec![rect(0.0, 0.0, 20.0, 200.0), rect(30.0, 0.0, 20.0, 150.0), rect(60.0, 0.0, 20.0, 90.0), rect(90.0, 0.0, 100.0, 20.0)];
        let (lying, ids) = axis(&q).unwrap();
        assert!(lying);
        assert_eq!(ids, vec![0, 1, 2]);
    }

    #[test]
    fn mixed_or_sparse_page_undecided() {
        // 반반 섞임
        let q = vec![rect(0.0, 0.0, 20.0, 200.0), rect(30.0, 0.0, 20.0, 150.0), rect(0.0, 0.0, 200.0, 20.0), rect(0.0, 30.0, 150.0, 20.0)];
        assert!(axis(&q).is_none());
        // 줄 모양 상자가 둘뿐
        let q = vec![rect(0.0, 0.0, 200.0, 20.0), rect(0.0, 30.0, 150.0, 20.0), rect(0.0, 60.0, 30.0, 25.0)];
        assert!(axis(&q).is_none());
    }

    // 비스듬한(45°) 줄은 축 판정에서 빠진다 — 부채꼴 배치 쪽은 판단하지 않는다
    #[test]
    fn diagonal_lines_are_ignored() {
        let diag = |x: f32, y: f32| -> [P; 4] { [[x, y], [x + 141.0, y + 141.0], [x + 127.0, y + 155.0], [x - 14.0, y + 14.0]] };
        let q = vec![diag(0.0, 0.0), diag(0.0, 50.0), diag(0.0, 100.0), diag(0.0, 150.0)];
        assert!(axis(&q).is_none());
        // 10° 기운 정도는 그대로 센다
        assert!(on_axis(&[[0.0, 0.0], [200.0, 35.0], [196.0, 55.0], [-4.0, 20.0]], false));
    }

    #[test]
    fn head_keeps_left_part() {
        let mut g = Bgr::white(10, 2);
        g.data[0] = 7;
        let h = head(&g, 4);
        assert_eq!((h.w, h.h), (4, 2));
        assert_eq!(h.data[0], 7);
        assert_eq!(head(&g, 50).w, 10);
    }

    #[test]
    fn flip_needs_clear_margin() {
        // 실측처럼 바른 방향 0.95 · 틀린 방향 0.7 → 뒤집기
        assert!(should_flip(&[0.7, 0.65, 0.72], &[0.95, 0.97, 0.9]));
        // 차가 작으면 지금 방향
        assert!(!should_flip(&[0.9, 0.92, 0.91], &[0.95, 0.96, 0.94]));
        // 둘 다 낮으면(글자 아님) 지금 방향
        assert!(!should_flip(&[0.2, 0.3, 0.25], &[0.45, 0.5, 0.4]));
        // 지금 방향이 더 좋으면 그대로
        assert!(!should_flip(&[0.95, 0.97], &[0.6, 0.62]));
    }
}
