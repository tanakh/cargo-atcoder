const BASE64_TBL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(v: &[u8]) -> String {
    let mut ret = String::new();
    let mut i = 0;
    while i < v.len() {
        ret.push(BASE64_TBL[(v[i] >> 2) as usize] as char);
        let t = (v[i] & 0x3) << 4;
        i += 1;
        if i >= v.len() {
            ret.push(BASE64_TBL[t as usize] as char);
            ret.push('=');
            ret.push('=');
            break;
        }
        ret.push(BASE64_TBL[(t | (v[i] >> 4)) as usize] as char);
        let t = (v[i] & 0xf) << 2;
        i += 1;
        if i >= v.len() {
            ret.push(BASE64_TBL[t as usize] as char);
            ret.push('=');
            break;
        }
        ret.push(BASE64_TBL[(t | (v[i] >> 6)) as usize] as char);
        ret.push(BASE64_TBL[(v[i] & 0x3f) as usize] as char);
        i += 1;
    }
    ret
}

#[test]
fn test_encode() {
    let txt = b"ABCDEFG";
    assert_eq!("QUJDREVGRw==", encode(txt));
}

pub fn decode(v: &str) -> Vec<u8> {
    let mut tbl = vec![64; 256];
    for (i, c) in BASE64_TBL.iter().enumerate() {
        tbl[*c as usize] = i as u8;
    }
    let mut ret = vec![];
    let mut buf = 0_u8;
    for (i, c) in v.bytes().enumerate() {
        if c == b'=' {
            break;
        }
        let c = tbl[c as usize];
        if c == 64 {
            continue;
        }
        match i % 4 {
            0 => {
                buf = c << 2;
            }
            1 => {
                ret.push(buf | c >> 4);
                buf = c << 4;
            }
            2 => {
                ret.push(buf | c >> 2);
                buf = c << 6;
            }
            3 => {
                ret.push(buf | c);
            }
            _ => unreachable!(),
        }
    }
    ret
}

#[test]
fn test_decode() {
    let b64 = "QUJDREVGRw==";
    assert_eq!(b"ABCDEFG", decode(b64).as_slice());
}
