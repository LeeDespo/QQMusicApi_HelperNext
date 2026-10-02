//! `musics.fcg` 的签名（`zzc`）。
//!
//! 参考库里 `sign=True` 的端点（乐谱、不喜欢列表等）走
//! `POST https://u.y.qq.com/cgi-bin/musics.fcg?sign=...&_=...`：请求体是同一份
//! `{comm, req_0}` 信封，但 URL 上要带一个 `zzc` 签名——服务端**按收到的字节**
//! 校验（签名对不上回 `2000`），所以发送与签名必须用同一份字节。
//!
//! 算法移植自 `qqmusic_api/algorithms/sign.py`：SHA-1 十六进制串上取两段固定下标，
//! 再与固定数组异或后 base64、去掉 `\\ / + =` 四类字符。它没有标准名字，
//! 照抄是唯一正确的做法——这不是"某种 DES/RSA"，自己发明等价物会直接失败。

use base64::Engine;
use sha1::{Digest, Sha1};

/// SHA-1 十六进制串（大写）里第一段的字符下标。
const PART_1_INDEXES: [usize; 7] = [23, 14, 6, 36, 16, 7, 19];
/// 第二段的字符下标。
const PART_2_INDEXES: [usize; 8] = [16, 1, 32, 12, 19, 27, 8, 5];
/// 与 SHA-1 字节异或的固定数组。
const SCRAMBLE_VALUES: [u8; 20] = [
    89, 39, 179, 150, 218, 82, 58, 252, 177, 52, 186, 123, 120, 64, 242, 133, 143, 161, 121, 179,
];

/// 计算 `zzc` 签名。输入是即将发送的**那串字节**，不是重新序列化的对象。
pub fn zzc_sign(payload: &[u8]) -> String {
    let digest = Sha1::digest(payload);
    let mut hash_hex = String::with_capacity(40);
    for byte in digest {
        hash_hex.push_str(&format!("{byte:02X}"));
    }
    let bytes = hash_hex.as_bytes();
    let part_1: String = PART_1_INDEXES
        .iter()
        .map(|index| bytes[*index] as char)
        .collect();
    let part_2: String = PART_2_INDEXES
        .iter()
        .map(|index| bytes[*index] as char)
        .collect();
    let mut part_3 = [0u8; 20];
    for (index, value) in SCRAMBLE_VALUES.iter().enumerate() {
        let pair = u8::from_str_radix(&hash_hex[index * 2..index * 2 + 2], 16).unwrap_or(0);
        part_3[index] = value ^ pair;
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(part_3);
    let cleaned: String = encoded
        .chars()
        .filter(|ch| !matches!(ch, '\\' | '/' | '+' | '='))
        .collect();
    format!("zzc{part_1}{cleaned}{part_2}").to_lowercase()
}

/// 参考实现里 `override_comm=True` 的那种 `comm`：以匿名 h5 调用方身份发请求
/// （乐谱读取要的就是这个——`uin` 空、`g_tk` 是字面量 5381，不是账号的 g_tk）。
///
/// 放在这里而不是让每个领域自己拼：`g_tk` 用错不会有报错，只会得到空数据。
pub fn anonymous_h5_comm(platform: Option<&str>) -> serde_json::Value {
    let mut comm = serde_json::json!({
        "g_tk": 5381,
        "uin": "",
        "format": "json",
        "inCharset": "utf-8",
        "outCharset": "utf-8",
        "notice": 0,
        "needNewCode": 1,
    });
    if let Some(platform) = platform {
        comm["platform"] = serde_json::json!(platform);
    }
    comm
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 向量由参考实现（`qqmusic_api/algorithms/sign.py`）直接产出，逐字比对。
    #[test]
    fn the_reference_vectors_hold() {
        assert_eq!(
            zzc_sign(br#"{"a":1}"#),
            "zzc7746109xq501htmv4ipz7c8owlxfpihjm1fd695c7"
        );
        assert_eq!(
            zzc_sign(br#"{"comm":{"ct":24}}"#),
            "zzce52634cbcpispnllkwa6oyvlivnkbgmhts353ac54b"
        );
        assert_eq!(
            zzc_sign(b"hello"),
            "zzcfa14dde89n1iwax0l5rimr0qwjexceiov4daaee8d6"
        );
    }

    /// 它是 SHA-1 的函数：换一个字节，签名必变。
    #[test]
    fn a_different_payload_signs_differently() {
        assert_ne!(zzc_sign(b"{\"a\":1}"), zzc_sign(b"{\"a\":2}"));
        assert!(zzc_sign(b"{}").starts_with("zzc"));
        assert!(zzc_sign(b"{}")
            .chars()
            .all(|ch| !matches!(ch, '\\' | '/' | '+' | '=')));
    }
}

#[cfg(test)]
mod live_probes {
    use crate::credential::CredentialStore;
    use crate::upstream::{Call, Platform, Upstream};

    /// 真机验证签名路（需要登录凭据与网络，默认忽略）：
    /// `cargo test -- --ignored live_signed_route --nocapture`
    ///
    /// 参考库把"不喜欢"的读取放在签名路上；这里用同一个端点验证
    /// `call_signed` 的整条链（信封 + zzc + `musics.fcg`）真的通。
    #[test]
    #[ignore]
    fn live_signed_route() {
        let store = CredentialStore::for_directory(&crate::data_directory());
        let credential = store.load().expect("需要一份可用的凭据");
        let upstream = Upstream::new();
        let data = upstream
            .call_signed(
                &credential,
                crate::Class::Account,
                Platform::Web,
                Call {
                    module: "music.feedback.FeedbackBlack",
                    method: "GetDislikeList",
                    param: serde_json::json!({"Cmd": 3, "Page": 1}),
                },
                &[],
                None,
            )
            .expect("签名路应当可用");
        println!(
            "signed route data keys: {:?}",
            data.as_object().map(|o| o.keys().collect::<Vec<_>>())
        );
    }
}

#[cfg(test)]
mod live_sheet_probe {
    use crate::credential::CredentialStore;
    use crate::upstream::{Call, Platform, Upstream};

    /// 真机验证「自定义 comm + 签名」这条组合路（默认忽略）。
    #[test]
    #[ignore]
    fn live_sheet_route() {
        let store = CredentialStore::for_directory(&crate::data_directory());
        let credential = store.load().expect("需要一份可用的凭据");
        let upstream = Upstream::new();
        let data = upstream
            .call_signed(
                &credential,
                crate::Class::Read,
                Platform::Web,
                Call {
                    module: "music.mir.SheetMusicSvr",
                    method: "HasSheetMusic",
                    param: serde_json::json!({"songMid": "003w2xz20QlUZt"}),
                },
                &[],
                Some(super::anonymous_h5_comm(None)),
            )
            .expect("乐谱查询应当可用");
        println!("sheet route data: {data}");
    }
}
