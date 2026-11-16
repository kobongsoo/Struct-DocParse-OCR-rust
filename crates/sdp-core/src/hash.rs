//------------------------------------------------------------------
// 파일 내용 해시 (SimpleRAG indexer::content_hash 와 같은 값)
//=> OCR 캐시 열쇠다. SimpleRAG 와 글자 하나까지 같아야 옛 캐시를 다시 OCR 하지 않고 쓴다.
//------------------------------------------------------------------

use sha1::{Digest, Sha1};

//------------------------------------------------------------------
// 파일 내용 SHA-1 (16진수 소문자)
//=> 1MB 씩 읽어 해시에 넣는다(큰 PDF 도 메모리를 많이 쓰지 않게).
//
// -in: path = 파일 경로
//
// -out: Ok(40자 16진수)
// -out: error = 파일 읽기 실패면 io::Error
//------------------------------------------------------------------
pub fn content_hash(path: &str) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|x| format!("{:02x}", x)).collect())
}
