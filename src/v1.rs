// v1（0.2.x / 0.3.x）儲存格式的唯讀解析，供開機自動遷移使用。
//
// v1 佈局回顧：
//   單塊檔：隨機數量的 0xFF 前導填充，接著
//     [4B magic 0x01311AAB][4B 檔名長][4B 資料長][4B len CRC][檔名][4B 檔名 CRC][資料][4B 資料 CRC]
//     整段 payload ≤ 4088，尾端 8 bytes 恆為 0xFF
//   多塊檔：首塊從 0 開始塞 payload[0..4088]，尾端 [4B CONT_A][4B 下一塊位址]；
//     接續塊 [4B CONT_B][4B 上一塊位址][payload 續]，中間塊裝 4080、最後一塊裝剩餘
//
// 注意：v1 的資料 CRC 在 payload「尾端」，可能跨越 cluster 邊界（0.3.x 的讀取
// bug 就出在這），因此這裡一律先重組完整 payload 再切欄位，不逐塊解讀。

use crate::store::{word_at, CLUSTER_SIZE};

pub(crate) const V1_MAGIC: u32 = 0x01311AAB;
pub(crate) const V1_CONT_A: u32 = 0x01311AAC;
pub(crate) const V1_CONT_B: u32 = 0x01311AAD;

const V1_DATA_AREA_END: usize = CLUSTER_SIZE - 8; // 4088
pub(crate) const V1_FIRST_CAPACITY: usize = V1_DATA_AREA_END; // 首塊可裝的 payload
pub(crate) const V1_CONT_CAPACITY: usize = V1_DATA_AREA_END - 8; // 接續塊可裝的 payload

pub(crate) struct V1Header {
    /// magic 在首塊內的起點（前面可能有隨機 0xFF 填充）
    pub start: usize,
    pub name_len: usize,
    pub data_len: usize,
}

impl V1Header {
    /// 完整 payload 長度（header 16 + 檔名 + 檔名 CRC 4 + 資料 + 資料 CRC 4）
    pub fn payload_len(&self) -> usize {
        24 + self.name_len + self.data_len
    }

    /// 這個檔案佔幾個 cluster（v1 的計算方式：首塊 4088、之後每塊 4080）
    pub fn need_clusters(&self) -> u32 {
        let p = self.payload_len() as u64;
        if p <= V1_FIRST_CAPACITY as u64 {
            1
        } else {
            1 + (p - V1_FIRST_CAPACITY as u64).div_ceil(V1_CONT_CAPACITY as u64) as u32
        }
    }
}

/// 跳過前導 0xFF 找 v1 magic 的起點（v1 單塊檔有隨機前導填充）
pub(crate) fn find_v1_file_start(buf: &[u8; CLUSTER_SIZE]) -> Option<usize> {
    let mut i = 0usize;
    while i < buf.len() && buf[i] == 0xFF {
        i += 1;
    }
    if i + 4 > buf.len() {
        return None;
    }
    if word_at(buf, i) == V1_MAGIC {
        Some(i)
    } else {
        None
    }
}

/// 解析並驗證 v1 首塊的 header（長度欄位 CRC、檔名 CRC、UTF-8、邊界）。
/// 任何一項不對就回 None（視為損毀，v1 自己也讀不回來的檔案）。
pub(crate) fn parse_v1_header(
    buf: &[u8; CLUSTER_SIZE],
    start: usize,
    partition_size: usize,
) -> Option<V1Header> {
    if start + 20 > CLUSTER_SIZE {
        return None;
    }
    let name_len = word_at(buf, start + 4) as usize;
    let data_len = word_at(buf, start + 8) as usize;

    // 長度欄位的 CRC（v1 對 name_len‖data_len 8 bytes 做 CRC32）
    let mut len_block = [0u8; 8];
    len_block[0..4].copy_from_slice(&(name_len as u32).to_be_bytes());
    len_block[4..8].copy_from_slice(&(data_len as u32).to_be_bytes());
    if crate::store::crc32(&len_block) != word_at(buf, start + 12) {
        return None;
    }

    // v1 讀取端假設「檔名與檔名 CRC 都在首塊內」，超出的一律視為無法解析
    let name_end = start + 16 + name_len;
    if name_end + 4 > V1_DATA_AREA_END {
        return None;
    }
    if data_len > partition_size {
        return None;
    }
    if crate::store::crc32(&buf[start + 16..name_end]) != word_at(buf, name_end) {
        return None;
    }
    core::str::from_utf8(&buf[start + 16..name_end]).ok()?;

    let header = V1Header {
        start,
        name_len,
        data_len,
    };
    // 有填充（start > 0）的一定是單塊檔；多塊檔首塊必從 0 開始
    if header.need_clusters() > 1 && start != 0 {
        return None;
    }
    Some(header)
}

/// 從重組好的 payload 取出檔名與資料，並驗證資料 CRC。
/// 回傳 (檔名, 資料範圍)；CRC 不符回 None。
pub(crate) fn split_v1_payload(payload: &[u8], header: &V1Header) -> Option<(usize, usize)> {
    let name_end = 16 + header.name_len;
    let data_start = name_end + 4;
    let data_end = data_start + header.data_len;
    if data_end + 4 > payload.len() {
        return None;
    }
    if crate::store::crc32(&payload[data_start..data_end]) != word_at(payload, data_end) {
        return None;
    }
    Some((data_start, data_end))
}
