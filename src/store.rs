use alloc::string::String;
use alloc::vec::Vec;
use core::str;
use embedded_storage::Storage;

use crate::v1;

#[cfg(feature = "esp")]
use esp_println::{print, println};
#[cfg(feature = "esp")]
use esp_storage::FlashStorage;

// ============================ 儲存格式（v2） ============================
// 每個 cluster 固定 4096 bytes，尾端 8 bytes 保留給接續標記：
//   [4088..4092) = CONT_A（此塊還有下一塊時才寫入）
//   [4092..4096) = 下一塊的絕對位址
// 資料區一律只到 4088，因此檔案內容永遠不會與接續標記混淆。
//
// 首塊（檔案開頭）：
//   [0..4)        MAGIC
//   [4..8)        檔名長度（big-endian）
//   [8..12)       資料長度（big-endian）
//   [12..16)      generation（覆寫世代序號，越大越新）
//   [16..20)      header CRC32（涵蓋 [4..16)）
//   [20..20+L)    檔名（UTF-8）
//   [20+L..24+L)  檔名 CRC32
//   [24+L..28+L)  資料 CRC32（寫入前就算好，放在 header 區，
//                 讀取端因此不需要處理「CRC 跨 cluster 邊界」的情況）
//   [28+L..4088)  資料
//
// 接續塊：
//   [0..4)   CONT_B
//   [4..8)   前一塊的絕對位址
//   [8..4088) 資料
//
// 寫入順序：接續塊由後往前寫，首塊（含 MAGIC 的 header）最後寫；
// 中途斷電只會留下沒有 header 的孤兒接續塊，開機時 heal() 會回收。
// 覆寫是「先寫新檔、再刪舊檔」，斷電後同名兩份由 generation 分辨新舊。
//
// 與 0.3.x 的格式不相容（magic 已更換），舊資料會被視為可用空間。

pub(crate) const CLUSTER_SIZE: usize = 4096;
const DATA_AREA_END: usize = CLUSTER_SIZE - 8;
const MAGIC: u32 = 0x01312AAB;
const CONT_A: u32 = 0x01312AAC;
const CONT_B: u32 = 0x01312AAD;
const HEADER_FIXED: usize = 20;
const FIRST_META: usize = HEADER_FIXED + 8; // header 固定區 + 檔名 CRC + 資料 CRC（不含檔名本體）
const CONT_DATA_START: usize = 8;
const CONT_CAPACITY: usize = DATA_AREA_END - CONT_DATA_START; // 4080

/// Maximum file name length in bytes (the header, name and CRCs must fit in the first cluster).
pub const MAX_NAME_LEN: usize = DATA_AREA_END - FIRST_META; // 4060

/// Errors returned by [`Store`] operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreError {
    /// No file with the given name exists.
    NotFound,
    /// The stored data failed validation (broken chain, CRC mismatch, torn write...).
    Corrupted,
    /// Not enough free clusters. Note that overwriting keeps the old copy until the
    /// new one is fully written, so both must fit at the same time.
    NoSpace,
    /// The file name exceeds [`MAX_NAME_LEN`] bytes.
    NameTooLong,
    /// `read()` was used on content that is not valid UTF-8 (use `read_bytes()` instead).
    NotUtf8,
    /// The underlying storage returned an error.
    Storage,
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self {
            StoreError::NotFound => "file not found",
            StoreError::Corrupted => "data corrupted",
            StoreError::NoSpace => "not enough free space",
            StoreError::NameTooLong => "file name too long",
            StoreError::NotUtf8 => "content is not valid UTF-8",
            StoreError::Storage => "flash access error",
        };
        f.write_str(s)
    }
}

// ============================ 純函式（不碰 flash） ============================

pub(crate) fn word_at(buf: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(buf[offset..offset + 4].try_into().unwrap())
}

fn crc32_init() -> u32 {
    0xFFFF_FFFF
}

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = if (crc & 1) != 0 { 0xEDB8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    crc
}

fn crc32_finalize(crc: u32) -> u32 {
    !crc
}

pub(crate) fn crc32(data: &[u8]) -> u32 {
    crc32_finalize(crc32_update(crc32_init(), data))
}

fn gen_newer(a: u32, b: u32) -> bool {
    // wrapping 比較：a 是否比 b 新
    a.wrapping_sub(b) as i32 > 0
}

fn first_data_capacity(name_len: usize) -> usize {
    // 首塊扣掉 header 與檔名後可放的資料量
    DATA_AREA_END - FIRST_META - name_len
}

fn need_clusters(name_len: usize, data_len: usize) -> u32 {
    let cap0 = first_data_capacity(name_len) as u64;
    let d = data_len as u64;
    if d <= cap0 {
        1
    } else {
        1 + (d - cap0).div_ceil(CONT_CAPACITY as u64) as u32
    }
}

fn data_span(name_len: usize, data_len: usize, i: usize) -> (usize, usize) {
    // 第 i 個 cluster 應存放的資料範圍（以檔案內位移表示）
    let cap0 = first_data_capacity(name_len);
    if i == 0 {
        (0, core::cmp::min(data_len, cap0))
    } else {
        let s = cap0 + (i - 1) * CONT_CAPACITY;
        (s, core::cmp::min(data_len, s + CONT_CAPACITY))
    }
}

fn header_name(buf: &[u8; CLUSTER_SIZE], name_len: usize) -> Option<&str> {
    str::from_utf8(&buf[HEADER_FIXED..HEADER_FIXED + name_len]).ok()
}

// ============================ 內部型別 ============================

struct ParsedHeader {
    name_len: usize,
    data_len: usize,
    generation: u32,
    data_crc: u32,
}

impl ParsedHeader {
    fn data_start(&self) -> usize {
        FIRST_META + self.name_len
    }
}

struct FoundFile {
    first_cluster: u32,
    header: ParsedHeader,
}

enum ChainNext {
    Next(u32),
    End,
    Broken,
}

// ============================ Store ============================

/// A tiny file store on top of any [`embedded_storage::Storage`] backend.
pub struct Store<S> {
    flash: S,
    flash_addr: u32,
    flash_size: u32,
    cluster: [u8; CLUSTER_SIZE],
    cluster_max_quantity: u32,
    write_cursor: u32,
    /// 尚未遷移完成的 v1（0.2.x/0.3.x）區塊地圖；空表示沒有 v1 資料
    v1_used: Vec<bool>,
}

impl<S: Storage> Store<S> {
    /// Create a store on `flash`, managing `flash_size` bytes starting at `flash_addr`.
    ///
    /// `flash_addr` must be 4096-aligned and `flash_size` at least 4096; the size is
    /// rounded down to a multiple of 4096. On construction the whole area is scanned:
    /// leftovers from interrupted writes are reclaimed, duplicated files (power loss
    /// during overwrite) are resolved to the newest generation, and files written by
    /// easy_store 0.2.x / 0.3.x are automatically migrated to the current format
    /// (copy first, erase after — safe to lose power at any point).
    pub fn new(flash: S, flash_addr: u32, flash_size: u32) -> Self {
        assert!(
            flash_size >= CLUSTER_SIZE as u32,
            "easy_store: flash_size 至少要 4096 bytes（一個 cluster）"
        );
        assert!(
            flash_addr.is_multiple_of(CLUSTER_SIZE as u32),
            "easy_store: flash_addr 必須對齊 4096"
        );
        let cluster_max_quantity = flash_size / CLUSTER_SIZE as u32;
        let mut store = Self {
            flash,
            flash_addr,
            flash_size,
            cluster: [0xFF; CLUSTER_SIZE],
            cluster_max_quantity,
            write_cursor: 0,
            v1_used: Vec::new(),
        };
        store.scan_v1(); // 標記 0.2.x/0.3.x 的舊格式區塊（遷移完成前一律視為已佔用）
        store.heal(); // 回收 v2 的斷電殘留，先騰出最大空間
        store.write_cursor = store.init_cursor();
        store.migrate_v1(); // 把 v1 檔案自動搬成 v2（copy-then-erase，斷電可續跑）
        store
    }

    /// Direct access to the underlying storage (advanced use / tests).
    pub fn storage_mut(&mut self) -> &mut S {
        &mut self.flash
    }

    /// Consume the store and return the underlying storage.
    pub fn into_storage(self) -> S {
        self.flash
    }

    // ============================ 低階 flash 存取 ============================

    fn cluster_addr(&self, cluster_number: u32) -> u32 {
        self.flash_addr + CLUSTER_SIZE as u32 * cluster_number
    }

    fn read_cluster(&mut self, cluster_number: u32, buf: &mut [u8; CLUSTER_SIZE]) -> Result<(), StoreError> {
        let addr = self.cluster_addr(cluster_number);
        self.flash.read(addr, buf).map_err(|_| StoreError::Storage)
    }

    fn read_first_word(&mut self, cluster_number: u32) -> Result<u32, StoreError> {
        let mut b = [0u8; 4];
        let addr = self.cluster_addr(cluster_number);
        self.flash.read(addr, &mut b).map_err(|_| StoreError::Storage)?;
        Ok(u32::from_be_bytes(b))
    }

    fn read_tail(&mut self, cluster_number: u32) -> Result<[u8; 8], StoreError> {
        // 讀 cluster 尾端 8 bytes 的接續標記（不用整塊讀進來）
        let mut b = [0u8; 8];
        let addr = self.cluster_addr(cluster_number) + DATA_AREA_END as u32;
        self.flash.read(addr, &mut b).map_err(|_| StoreError::Storage)?;
        Ok(b)
    }

    fn flush_scratch(&mut self, cluster_number: u32) -> Result<(), StoreError> {
        let addr = self.cluster_addr(cluster_number);
        self.flash.write(addr, &self.cluster).map_err(|_| StoreError::Storage)
    }

    fn erase_cluster(&mut self, cluster_number: u32) -> Result<(), StoreError> {
        self.cluster.fill(0xFF);
        self.flush_scratch(cluster_number)
    }

    fn addr_to_cluster_index(&self, addr: u32) -> Option<u32> {
        if addr < self.flash_addr {
            return None;
        }
        let offset = addr - self.flash_addr;
        if !offset.is_multiple_of(CLUSTER_SIZE as u32) {
            return None;
        }
        let idx = offset / CLUSTER_SIZE as u32;
        if idx >= self.cluster_max_quantity {
            return None;
        }
        Some(idx)
    }

    // ============================ cluster 判讀 ============================

    fn check_used(&mut self, cluster_number: u32) -> bool {
        // 尚未遷移的 v1 區塊也算已佔用，任何寫入都不會蓋到舊資料
        if self
            .v1_used
            .get(cluster_number as usize)
            .copied()
            .unwrap_or(false)
        {
            return true;
        }
        match self.read_first_word(cluster_number) {
            Ok(w) => w == MAGIC || w == CONT_B,
            Err(_) => true, // 讀取失敗時保守視為已使用，避免蓋到內容
        }
    }

    fn parse_header(&self, buf: &[u8; CLUSTER_SIZE]) -> Option<ParsedHeader> {
        if word_at(buf, 0) != MAGIC {
            return None;
        }
        let name_len = word_at(buf, 4) as usize;
        let data_len = word_at(buf, 8) as usize;
        let generation = word_at(buf, 12);
        if crc32(&buf[4..16]) != word_at(buf, 16) {
            return None;
        }
        if name_len > MAX_NAME_LEN {
            return None;
        }
        if data_len > self.flash_size as usize {
            return None;
        }
        let name_end = HEADER_FIXED + name_len;
        if crc32(&buf[HEADER_FIXED..name_end]) != word_at(buf, name_end) {
            return None;
        }
        str::from_utf8(&buf[HEADER_FIXED..name_end]).ok()?;
        let data_crc = word_at(buf, name_end + 4);
        Some(ParsedHeader {
            name_len,
            data_len,
            generation,
            data_crc,
        })
    }

    fn chain_next(&self, tail: &[u8]) -> ChainNext {
        // tail 為 cluster 尾端 8 bytes：[標記 4B][下一塊位址 4B]
        if word_at(tail, 0) != CONT_A {
            return ChainNext::End;
        }
        match self.addr_to_cluster_index(word_at(tail, 4)) {
            Some(idx) => ChainNext::Next(idx),
            None => ChainNext::Broken,
        }
    }

    // ============================ 檔案搜尋 ============================

    fn find_files(&mut self, file_name: &str) -> Vec<FoundFile> {
        let mut out: Vec<FoundFile> = Vec::new();
        let mut buf = [0u8; CLUSTER_SIZE];
        for i in 0..self.cluster_max_quantity {
            match self.read_first_word(i) {
                Ok(w) if w == MAGIC => {}
                _ => continue,
            }
            if self.read_cluster(i, &mut buf).is_err() {
                continue;
            }
            let Some(header) = self.parse_header(&buf) else {
                continue;
            };
            if header_name(&buf, header.name_len) != Some(file_name) {
                continue;
            }
            out.push(FoundFile {
                first_cluster: i,
                header,
            });
        }
        out
    }

    fn newest_file(&mut self, file_name: &str) -> Result<FoundFile, StoreError> {
        // 同名多份（覆寫途中斷電的殘留）時取 generation 最新的
        let mut best: Option<FoundFile> = None;
        for f in self.find_files(file_name) {
            best = match best {
                None => Some(f),
                Some(b) => {
                    if gen_newer(f.header.generation, b.header.generation) {
                        Some(f)
                    } else {
                        Some(b)
                    }
                }
            };
        }
        best.ok_or(StoreError::NotFound)
    }

    fn collect_chain(&mut self, first_cluster: u32) -> Vec<u32> {
        // 盡力收集一條鏈上的所有 cluster（供刪除用；鏈斷了就收到哪算哪）
        let mut chain = alloc::vec![first_cluster];
        let mut cur = first_cluster;
        while let Ok(tail) = self.read_tail(cur) {
            match self.chain_next(&tail) {
                ChainNext::Next(nx)
                    if !chain.contains(&nx) && (chain.len() as u32) < self.cluster_max_quantity =>
                {
                    chain.push(nx);
                    cur = nx;
                }
                _ => break,
            }
        }
        chain
    }

    fn validate_chain(&mut self, first_cluster: u32, header: &ParsedHeader) -> Option<Vec<u32>> {
        // 結構驗證：鏈長要與 header 宣告的資料量一致、不能斷、不能成環
        let expected = need_clusters(header.name_len, header.data_len);
        let mut chain = alloc::vec![first_cluster];
        let mut cur = first_cluster;
        loop {
            let tail = self.read_tail(cur).ok()?;
            match self.chain_next(&tail) {
                ChainNext::End => break,
                ChainNext::Broken => return None,
                ChainNext::Next(nx) => {
                    if chain.len() as u32 >= expected || chain.contains(&nx) {
                        return None;
                    }
                    if self.read_first_word(nx).ok()? != CONT_B {
                        return None;
                    }
                    chain.push(nx);
                    cur = nx;
                }
            }
        }
        if chain.len() as u32 == expected {
            Some(chain)
        } else {
            None
        }
    }

    // ============================ 空間管理 ============================

    fn find_free_cluster_excluding(&mut self, start: u32, taken: &[u32]) -> Option<u32> {
        // 從 start 繞一圈找 free cluster；taken 是本次寫入已分配、還沒落地的區塊
        let max = self.cluster_max_quantity;
        let mut n = start % max;
        for _ in 0..max {
            if !taken.contains(&n) && !self.check_used(n) {
                return Some(n);
            }
            n = (n + 1) % max;
        }
        None
    }

    fn allocate_clusters(&mut self, quantity: u32) -> Option<Vec<u32>> {
        // 從 cursor 出發輪流分配，寫入點分散到整個分區以平均磨損；
        // 空間不足回 None，什麼都不會寫入
        let mut clusters: Vec<u32> = Vec::new();
        let mut cursor = self.write_cursor;
        for _ in 0..quantity {
            let c = self.find_free_cluster_excluding(cursor, &clusters)?;
            cursor = (c + 1) % self.cluster_max_quantity;
            clusters.push(c);
        }
        self.write_cursor = cursor;
        Some(clusters)
    }

    fn init_cursor(&mut self) -> u32 {
        // 取「最大已用 cluster index + 1」作為起點（mod max），無檔案則回 0
        // 重開機後從這裡接著寫，磨損不會每次都從低位址開始累積
        let mut highest: Option<u32> = None;
        for i in 0..self.cluster_max_quantity {
            if self.check_used(i) {
                highest = Some(i);
            }
        }
        match highest {
            Some(h) => (h + 1) % self.cluster_max_quantity,
            None => 0,
        }
    }

    fn count_used_clusters(&mut self) -> u32 {
        let mut count: u32 = 0;
        for i in 0..self.cluster_max_quantity {
            if self.check_used(i) {
                count += 1;
            }
        }
        count
    }

    /// Total managed space in bytes (multiple of 4096).
    pub fn capacity(&self) -> u32 {
        self.cluster_max_quantity * CLUSTER_SIZE as u32
    }

    /// Space taken by used clusters, in bytes.
    pub fn used_space(&mut self) -> u32 {
        self.count_used_clusters() * CLUSTER_SIZE as u32
    }

    /// Space still available for new clusters, in bytes.
    pub fn free_space(&mut self) -> u32 {
        self.capacity() - self.used_space()
    }

    // ============================ 寫入 ============================

    /// Store a UTF-8 text file. Overwrites any existing file with the same name.
    pub fn write(&mut self, file_name: &str, file_data: &str) -> Result<(), StoreError> {
        self.write_bytes(file_name, file_data.as_bytes())
    }

    /// Store arbitrary binary data. Overwrites any existing file with the same name.
    ///
    /// The old copy is deleted only after the new one is fully written, so a power
    /// loss never destroys the previous version — but it also means an overwrite
    /// needs enough free space for both copies at the same time.
    pub fn write_bytes(&mut self, file_name: &str, file_data: &[u8]) -> Result<(), StoreError> {
        if file_name.len() > MAX_NAME_LEN {
            return Err(StoreError::NameTooLong);
        }
        if file_data.len() as u64 > u32::MAX as u64 {
            return Err(StoreError::NoSpace);
        }
        let existing = self.find_files(file_name);
        let generation = existing
            .iter()
            .map(|f| f.header.generation)
            .reduce(|a, b| if gen_newer(b, a) { b } else { a })
            .map(|g| g.wrapping_add(1))
            .unwrap_or(0);
        let quantity = need_clusters(file_name.len(), file_data.len());
        let Some(clusters) = self.allocate_clusters(quantity) else {
            return Err(StoreError::NoSpace);
        };
        self.save_file(file_name, file_data, generation, &clusters)?;
        // 新檔完整落地後才清舊檔
        for f in &existing {
            self.delete_found(f)?;
        }
        Ok(())
    }

    fn save_file(
        &mut self,
        file_name: &str,
        file_data: &[u8],
        generation: u32,
        clusters: &[u32],
    ) -> Result<(), StoreError> {
        let name = file_name.as_bytes();
        let data_crc = crc32(file_data);
        let n = clusters.len();
        // 由後往前寫：首塊（header）最後落地，中途斷電只會留下孤兒接續塊
        for i in (0..n).rev() {
            self.cluster.fill(0xFF);
            let (span_s, span_e) = data_span(name.len(), file_data.len(), i);
            let chunk = &file_data[span_s..span_e];
            if i == 0 {
                self.cluster[0..4].copy_from_slice(&MAGIC.to_be_bytes());
                self.cluster[4..8].copy_from_slice(&(name.len() as u32).to_be_bytes());
                self.cluster[8..12].copy_from_slice(&(file_data.len() as u32).to_be_bytes());
                self.cluster[12..16].copy_from_slice(&generation.to_be_bytes());
                let header_crc = crc32(&self.cluster[4..16]);
                self.cluster[16..20].copy_from_slice(&header_crc.to_be_bytes());
                let name_end = HEADER_FIXED + name.len();
                self.cluster[HEADER_FIXED..name_end].copy_from_slice(name);
                self.cluster[name_end..name_end + 4].copy_from_slice(&crc32(name).to_be_bytes());
                self.cluster[name_end + 4..name_end + 8].copy_from_slice(&data_crc.to_be_bytes());
                let ds = FIRST_META + name.len();
                self.cluster[ds..ds + chunk.len()].copy_from_slice(chunk);
            } else {
                self.cluster[0..4].copy_from_slice(&CONT_B.to_be_bytes());
                let prev_addr = self.cluster_addr(clusters[i - 1]);
                self.cluster[4..8].copy_from_slice(&prev_addr.to_be_bytes());
                self.cluster[CONT_DATA_START..CONT_DATA_START + chunk.len()].copy_from_slice(chunk);
            }
            if i + 1 < n {
                let next_addr = self.cluster_addr(clusters[i + 1]);
                self.cluster[DATA_AREA_END..DATA_AREA_END + 4].copy_from_slice(&CONT_A.to_be_bytes());
                self.cluster[DATA_AREA_END + 4..CLUSTER_SIZE].copy_from_slice(&next_addr.to_be_bytes());
            }
            self.flush_scratch(clusters[i])?;
        }
        Ok(())
    }

    /// Append text to a file (created if it does not exist).
    pub fn append(&mut self, file_name: &str, file_data: &str) -> Result<(), StoreError> {
        self.append_bytes(file_name, file_data.as_bytes())
    }

    /// Append bytes to a file (created if it does not exist).
    ///
    /// Note: the existing content passes through RAM (read + rewrite).
    pub fn append_bytes(&mut self, file_name: &str, file_data: &[u8]) -> Result<(), StoreError> {
        match self.read_bytes(file_name) {
            Ok(mut old) => {
                old.extend_from_slice(file_data);
                self.write_bytes(file_name, &old)
            }
            Err(StoreError::NotFound) => self.write_bytes(file_name, file_data),
            Err(e) => Err(e),
        }
    }

    /// Rename `from` to `to`, replacing `to` if it exists.
    ///
    /// Note: the content passes through RAM (read + rewrite). On power loss the
    /// worst case is that both names exist; the content is never lost.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<(), StoreError> {
        if from == to {
            return if self.exists(from) {
                Ok(())
            } else {
                Err(StoreError::NotFound)
            };
        }
        let data = self.read_bytes(from)?;
        self.write_bytes(to, &data)?;
        self.delete(from)
    }

    // ============================ 讀取 ============================

    /// Read a file as UTF-8 text.
    pub fn read(&mut self, file_name: &str) -> Result<String, StoreError> {
        String::from_utf8(self.read_bytes(file_name)?).map_err(|_| StoreError::NotUtf8)
    }

    /// Read a whole file. The content is verified against the stored CRC32.
    pub fn read_bytes(&mut self, file_name: &str) -> Result<Vec<u8>, StoreError> {
        let f = self.newest_file(file_name)?;
        let expected = need_clusters(f.header.name_len, f.header.data_len);
        let mut out: Vec<u8> = Vec::with_capacity(f.header.data_len);
        let mut crc = crc32_init();
        let mut remaining = f.header.data_len;
        let mut buf = [0u8; CLUSTER_SIZE];
        let mut data_start = f.header.data_start();
        let mut visited: u32 = 1;
        let mut cur = f.first_cluster;
        self.read_cluster(cur, &mut buf)?;
        loop {
            let avail = DATA_AREA_END - data_start;
            let take = core::cmp::min(remaining, avail);
            let chunk = &buf[data_start..data_start + take];
            out.extend_from_slice(chunk);
            crc = crc32_update(crc, chunk);
            remaining -= take;
            match self.chain_next(&buf[DATA_AREA_END..CLUSTER_SIZE]) {
                ChainNext::End => break,
                ChainNext::Broken => return Err(StoreError::Corrupted),
                ChainNext::Next(nx) => {
                    if remaining == 0 || visited >= expected {
                        return Err(StoreError::Corrupted);
                    }
                    visited += 1;
                    cur = nx;
                    self.read_cluster(cur, &mut buf)?;
                    if word_at(&buf, 0) != CONT_B {
                        return Err(StoreError::Corrupted);
                    }
                    data_start = CONT_DATA_START;
                }
            }
        }
        if remaining != 0 {
            return Err(StoreError::Corrupted);
        }
        if crc32_finalize(crc) != f.header.data_crc {
            return Err(StoreError::Corrupted);
        }
        Ok(out)
    }

    /// Read part of a file into `buf`, starting at byte `offset`.
    ///
    /// Returns the number of bytes copied (0 if `offset` is at or past the end).
    /// Useful for large files that do not fit in RAM. Unlike [`Store::read_bytes`],
    /// the content CRC is not verified (that would require reading the whole file).
    pub fn read_range(
        &mut self,
        file_name: &str,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, StoreError> {
        let f = self.newest_file(file_name)?;
        let data_len = f.header.data_len;
        if offset >= data_len || buf.is_empty() {
            return Ok(0);
        }
        let want = core::cmp::min(buf.len(), data_len - offset);
        let expected = need_clusters(f.header.name_len, data_len);
        let mut cbuf = [0u8; CLUSTER_SIZE];
        let mut cur = f.first_cluster;
        let mut visited: u32 = 1;
        let mut is_first = true;
        let mut file_pos: usize = 0; // 目前 cluster 資料區起點在檔案內的位移
        let mut copied: usize = 0;
        loop {
            let data_start = if is_first { f.header.data_start() } else { CONT_DATA_START };
            let take = core::cmp::min(data_len - file_pos, DATA_AREA_END - data_start);
            let seg_s = core::cmp::max(file_pos, offset);
            let seg_e = core::cmp::min(file_pos + take, offset + want);
            let tail: [u8; 8] = if seg_e > seg_s {
                // 與要讀的範圍有交集才整塊讀進來
                self.read_cluster(cur, &mut cbuf)?;
                if !is_first && word_at(&cbuf, 0) != CONT_B {
                    return Err(StoreError::Corrupted);
                }
                let src = data_start + (seg_s - file_pos);
                buf[seg_s - offset..seg_e - offset].copy_from_slice(&cbuf[src..src + (seg_e - seg_s)]);
                copied += seg_e - seg_s;
                cbuf[DATA_AREA_END..CLUSTER_SIZE].try_into().unwrap()
            } else {
                // 範圍外的 cluster 只讀尾端標記，快速跳過
                self.read_tail(cur)?
            };
            file_pos += take;
            if copied == want {
                return Ok(copied);
            }
            match self.chain_next(&tail) {
                ChainNext::End => break,
                ChainNext::Broken => return Err(StoreError::Corrupted),
                ChainNext::Next(nx) => {
                    if visited >= expected {
                        return Err(StoreError::Corrupted);
                    }
                    visited += 1;
                    cur = nx;
                    is_first = false;
                }
            }
        }
        Err(StoreError::Corrupted) // 鏈比 header 宣告的資料量還短
    }

    /// Size of a file in bytes (reads only the header cluster).
    pub fn file_size(&mut self, file_name: &str) -> Result<usize, StoreError> {
        Ok(self.newest_file(file_name)?.header.data_len)
    }

    /// Whether a file with exactly this name exists (`exists("/data")` is `false`
    /// even when `/data/foo.txt` exists).
    pub fn exists(&mut self, file_name: &str) -> bool {
        !self.find_files(file_name).is_empty()
    }

    /// List all stored file names.
    pub fn files(&mut self) -> Vec<String> {
        let mut result: Vec<String> = Vec::new();
        let mut buf = [0u8; CLUSTER_SIZE];
        for i in 0..self.cluster_max_quantity {
            match self.read_first_word(i) {
                Ok(w) if w == MAGIC => {}
                _ => continue,
            }
            if self.read_cluster(i, &mut buf).is_err() {
                continue;
            }
            let Some(header) = self.parse_header(&buf) else {
                continue;
            };
            let Some(name) = header_name(&buf, header.name_len) else {
                continue;
            };
            if !result.iter().any(|x| x == name) {
                result.push(String::from(name));
            }
        }
        result
    }

    /// List the direct children of `path` (like `std::fs::read_dir`); child
    /// directories are reported with a trailing `/`.
    pub fn read_dir(&mut self, path: &str) -> Vec<String> {
        // 模仿 std::fs::read_dir：只回直屬子項目
        // 檔案 → 完整路徑；下一層子目錄 → 用尾端 '/' 標示（例如 "/data/sub/"）
        let mut prefix = String::from(path);
        if !prefix.ends_with('/') {
            prefix.push('/');
        }
        let mut result: Vec<String> = Vec::new();
        for file in self.files() {
            let Some(rest) = file.strip_prefix(prefix.as_str()) else {
                continue;
            };
            let entry = match rest.find('/') {
                Some(idx) => {
                    let mut p = String::from(prefix.as_str());
                    p.push_str(&rest[..idx]);
                    p.push('/');
                    p
                }
                None => file.clone(),
            };
            if !result.contains(&entry) {
                result.push(entry);
            }
        }
        result
    }

    // ============================ 刪除 ============================

    fn delete_found(&mut self, f: &FoundFile) -> Result<(), StoreError> {
        // 先擦首塊（檔案立即「消失」），斷電殘留的接續塊由 heal() 回收
        let chain = self.collect_chain(f.first_cluster);
        for idx in chain {
            self.erase_cluster(idx)?;
        }
        Ok(())
    }

    /// Delete a file. Returns [`StoreError::NotFound`] if it does not exist.
    pub fn delete(&mut self, file_name: &str) -> Result<(), StoreError> {
        let found = self.find_files(file_name);
        if found.is_empty() {
            return Err(StoreError::NotFound);
        }
        for f in &found {
            self.delete_found(f)?;
        }
        Ok(())
    }

    /// Delete every file under `path` (recursively). Returns how many files were deleted.
    pub fn delete_dir(&mut self, path: &str) -> Result<u32, StoreError> {
        let mut prefix = String::from(path);
        if !prefix.ends_with('/') {
            prefix.push('/');
        }
        let mut count: u32 = 0;
        for name in self.files() {
            if name.starts_with(prefix.as_str()) {
                self.delete(&name)?;
                count += 1;
            }
        }
        Ok(count)
    }

    /// Delete every file in the store.
    pub fn delete_all_data(&mut self) -> Result<(), StoreError> {
        for i in 0..self.cluster_max_quantity {
            if self.check_used(i) {
                self.erase_cluster(i)?;
            }
        }
        self.v1_used = Vec::new(); // 未遷移的 v1 區塊也一併清掉了
        Ok(())
    }

    // ============================ 開機自我修復 ============================

    fn heal(&mut self) {
        // 回收斷電殘留：孤兒接續塊（header 尚未落地）、結構不完整的鏈；
        // 同名多份（覆寫途中斷電）只保留 generation 最新的一份
        let max = self.cluster_max_quantity as usize;
        let mut used = alloc::vec![false; max];
        let mut keep = alloc::vec![false; max];
        let mut kept: Vec<(String, u32, Vec<u32>)> = Vec::new();
        let mut buf = [0u8; CLUSTER_SIZE];
        for i in 0..self.cluster_max_quantity {
            let word = match self.read_first_word(i) {
                Ok(w) => w,
                Err(_) => {
                    // 讀不到的區塊不動它
                    used[i as usize] = true;
                    keep[i as usize] = true;
                    continue;
                }
            };
            if word != MAGIC && word != CONT_B {
                continue;
            }
            used[i as usize] = true;
            if word != MAGIC {
                continue;
            }
            if self.read_cluster(i, &mut buf).is_err() {
                keep[i as usize] = true;
                continue;
            }
            let Some(header) = self.parse_header(&buf) else {
                continue; // 壞 header → 稍後回收
            };
            let Some(name) = header_name(&buf, header.name_len) else {
                continue;
            };
            let Some(chain) = self.validate_chain(i, &header) else {
                continue; // 鏈不完整 → 稍後回收
            };
            match kept.iter().position(|k| k.0 == name) {
                None => kept.push((String::from(name), header.generation, chain)),
                Some(p) => {
                    if gen_newer(header.generation, kept[p].1) {
                        kept[p] = (String::from(name), header.generation, chain);
                    }
                }
            }
        }
        for (_, _, chain) in &kept {
            for &c in chain {
                keep[c as usize] = true;
            }
        }
        for i in 0..self.cluster_max_quantity {
            if used[i as usize] && !keep[i as usize] {
                let _ = self.erase_cluster(i); // 盡力而為
            }
        }
    }

    // ============================ v1 自動遷移 ============================

    fn scan_v1(&mut self) {
        // 辨識 0.2.x/0.3.x 格式的區塊。v1 單塊檔有隨機 0xFF 前導填充，
        // 所以開頭是 0xFF 的區塊要整塊讀進來確認是舊檔還是空區塊
        let max = self.cluster_max_quantity as usize;
        let mut map = alloc::vec![false; max];
        let mut any = false;
        let mut buf = [0u8; CLUSTER_SIZE];
        for i in 0..self.cluster_max_quantity {
            let Ok(word) = self.read_first_word(i) else {
                continue;
            };
            let is_v1 = if word == v1::V1_MAGIC || word == v1::V1_CONT_B {
                true
            } else if (word >> 24) as u8 == 0xFF {
                self.read_cluster(i, &mut buf).is_ok() && v1::find_v1_file_start(&buf).is_some()
            } else {
                false
            };
            if is_v1 {
                map[i as usize] = true;
                any = true;
            }
        }
        self.v1_used = if any { map } else { Vec::new() };
    }

    fn migrate_v1(&mut self) {
        // 把 v1 檔案逐一搬成 v2：先寫出完整的新複本、成功後才擦舊資料，
        // 任何時點斷電下次開機都能續跑（冪等）。搬不動的（空間或記憶體
        // 不足）原樣保留、下次再試；損毀的（v1 自己也讀不回來）直接回收
        if self.v1_used.is_empty() {
            return;
        }
        struct V1File {
            first: u32,
            header: v1::V1Header,
        }
        let mut found: Vec<V1File> = Vec::new();
        {
            let mut buf = [0u8; CLUSTER_SIZE];
            for i in 0..self.cluster_max_quantity {
                if !self.v1_used[i as usize] {
                    continue;
                }
                if self.read_cluster(i, &mut buf).is_err() {
                    continue;
                }
                if word_at(&buf, 0) == v1::V1_CONT_B {
                    continue; // 接續塊，由所屬檔案的鏈處理
                }
                let Some(start) = v1::find_v1_file_start(&buf) else {
                    continue;
                };
                if let Some(header) = v1::parse_v1_header(&buf, start, self.flash_size as usize) {
                    found.push(V1File { first: i, header });
                }
                // header 解析失敗的首塊留給掃尾回收
            }
        }
        found.sort_by_key(|f| f.header.data_len); // 小檔先搬，設定檔類必定先成功
        let mut keep = alloc::vec![false; self.cluster_max_quantity as usize];
        let mut any_kept = false;
        for f in &found {
            let chain = self.collect_v1_chain(f.first);
            if chain.len() as u32 != f.header.need_clusters() {
                continue; // 鏈不完整（v1 時代的斷電殘檔）→ 掃尾回收
            }
            let Some(payload) = self.read_v1_payload(&f.header, &chain) else {
                // flash 讀取錯誤或記憶體不足：原樣保留，下次開機再試
                for &c in &chain {
                    keep[c as usize] = true;
                }
                any_kept = true;
                continue;
            };
            let Some((data_s, data_e)) = v1::split_v1_payload(&payload, &f.header) else {
                continue; // 資料 CRC 不符 → 掃尾回收
            };
            let Ok(name) = str::from_utf8(&payload[16..16 + f.header.name_len]) else {
                continue;
            };
            if !self.exists(name) && self.write_bytes(name, &payload[data_s..data_e]).is_err() {
                // 通常是 NoSpace：保留 v1 原樣（check_used 會保護它們不被覆寫）
                for &c in &chain {
                    keep[c as usize] = true;
                }
                any_kept = true;
                continue;
            }
            // v2 已有同名完整檔（上次遷移在擦除前斷電）或剛寫入成功 → 擦掉 v1 原本
            self.erase_v1_chain(&chain);
        }
        // 掃尾：不屬於任何保留檔案的 v1 區塊（孤兒接續塊、損毀檔）一併回收
        for i in 0..self.cluster_max_quantity {
            if self.v1_used[i as usize] && !keep[i as usize] {
                let _ = self.erase_cluster(i);
                self.v1_used[i as usize] = false;
            }
        }
        if !any_kept {
            self.v1_used = Vec::new();
        }
    }

    fn collect_v1_chain(&mut self, first_cluster: u32) -> Vec<u32> {
        // 沿 v1 的 CONT_A 指標收集整條鏈（防環、防越界、驗證接續塊標記）
        let mut chain = alloc::vec![first_cluster];
        let mut cur = first_cluster;
        while let Ok(tail) = self.read_tail(cur) {
            if word_at(&tail, 0) != v1::V1_CONT_A {
                break;
            }
            let Some(nx) = self.addr_to_cluster_index(word_at(&tail, 4)) else {
                break;
            };
            if chain.contains(&nx) || chain.len() as u32 >= self.cluster_max_quantity {
                break;
            }
            let Ok(w) = self.read_first_word(nx) else {
                break;
            };
            if w != v1::V1_CONT_B {
                break;
            }
            chain.push(nx);
            cur = nx;
        }
        chain
    }

    fn read_v1_payload(&mut self, header: &v1::V1Header, chain: &[u32]) -> Option<Vec<u8>> {
        // 把 v1 的 payload 重組回連續的一段（v1 的資料 CRC 可能跨 cluster
        // 邊界，先重組再切欄位最保險）。記憶體不足時回 None、檔案原樣保留
        let total = header.payload_len();
        let mut payload: Vec<u8> = Vec::new();
        if payload.try_reserve_exact(total).is_err() {
            return None;
        }
        let mut buf = [0u8; CLUSTER_SIZE];
        for (k, &c) in chain.iter().enumerate() {
            if self.read_cluster(c, &mut buf).is_err() {
                return None;
            }
            let (start, cap) = if k == 0 {
                (header.start, v1::V1_FIRST_CAPACITY - header.start)
            } else {
                (8usize, v1::V1_CONT_CAPACITY)
            };
            let take = core::cmp::min(total - payload.len(), cap);
            payload.extend_from_slice(&buf[start..start + take]);
        }
        if payload.len() == total {
            Some(payload)
        } else {
            None
        }
    }

    fn erase_v1_chain(&mut self, chain: &[u32]) {
        // 先擦首塊（檔案先「消失」），再擦接續塊
        for &c in chain {
            let _ = self.erase_cluster(c);
            if let Some(m) = self.v1_used.get_mut(c as usize) {
                *m = false;
            }
        }
    }
}

// ============================ ESP 便利層 ============================

#[cfg(feature = "esp")]
impl Store<FlashStorage<'static>> {
    /// 便利建構子：直接收 `FLASH` peripheral，
    /// 等同 `Store::new(FlashStorage::new(flash), flash_addr, flash_size)`。
    pub fn new_esp(
        flash: esp_hal::peripherals::FLASH<'static>,
        flash_addr: u32,
        flash_size: u32,
    ) -> Self {
        Self::new(FlashStorage::new(flash), flash_addr, flash_size)
    }
}

#[cfg(feature = "esp")]
impl<S: Storage> Store<S> {
    /// 印出每個 cluster 的使用狀況（+ 已使用、- 未使用）。
    pub fn show_usage_cluster(&mut self) {
        println!("檢查目前檔案佔用了那些區塊↴");
        for i in 0..self.cluster_max_quantity {
            if self.check_used(i) {
                print!("+");
            } else {
                print!("-");
            }
        }
        println!("");
    }

    /// 印出總容量與使用容量。
    pub fn show_usage_capacity(&mut self) {
        let total = self.capacity();
        let used = self.used_space();
        Self::print_capacity("總容量", total);
        Self::print_capacity("使用容量", used);
    }

    fn print_capacity(label: &str, v: u32) {
        if v >= 1024 * 1024 * 1024 {
            println!("{} --> {} gb", label, v as f32 / 1024.0 / 1024.0 / 1024.0);
        } else if v >= 1024 * 1024 {
            println!("{} --> {} mb", label, v as f32 / 1024.0 / 1024.0);
        } else if v >= 1024 {
            println!("{} --> {} kb", label, v as f32 / 1024.0);
        } else {
            println!("{} --> {} byte", label, v);
        }
    }

    /// 印出指定檔名佔用了哪些區塊（+ 佔用、- 未佔用）。
    pub fn show_file_name_exist(&mut self, file_name: &str) {
        println!("檢查目前使用【{file_name}】的檔案佔用了那些區塊↴");
        let mut map = alloc::vec![false; self.cluster_max_quantity as usize];
        for f in self.find_files(file_name) {
            for idx in self.collect_chain(f.first_cluster) {
                map[idx as usize] = true;
            }
        }
        for m in map {
            if m {
                print!("+");
            } else {
                print!("-");
            }
        }
        println!("");
    }

    /// 印出路徑下的直屬子項目。
    pub fn show_read_dir(&mut self, path: &str) {
        println!("路徑【{:?}】下的檔案分別有↴", path);
        for name in self.read_dir(path) {
            println!("{:?}", name);
        }
    }

    /// 印出全部檔案名稱。
    pub fn show_all_data_name(&mut self) {
        println!("全部儲存的檔案名稱 ↴");
        for name in self.files() {
            println!("{:?}", name);
        }
    }
}
