use alloc::string::String;
use alloc::vec::Vec;
use core::str;
use embedded_storage::{ReadStorage, Storage};
use esp_hal::rng::Rng;
use esp_println::{print, println};
use esp_storage::FlashStorage;



const CLUSTER_SIZE: usize = 4096;
const CONCRETE_CLUSTER_SIZE: usize = CLUSTER_SIZE - 8;
pub struct Store {
    flash_addr: u32,
    flash_size: u32,
    cluster: [u8; CLUSTER_SIZE],
    magic_name: u32,
    cluster_max_quantity: u32,
    continued_A: u32,
    continued_B: u32,
}



impl Store {
    pub fn new(flash_addr:u32, flash_size: u32) -> Self {
        let cluster_max_quantity: u32 = flash_size/CLUSTER_SIZE as u32;
        Self {
            flash_addr: flash_addr,
            flash_size: flash_size,
            cluster: [0xFF; CLUSTER_SIZE],
            magic_name: 0x01311AAB,
            cluster_max_quantity: cluster_max_quantity,
            continued_A: 0x01311AAC,
            continued_B: 0x01311AAD,
        }
    }
    fn crc32(&self, data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = if (crc & 1) != 0 { 0xEDB8_8320 } else { 0 };
                crc = (crc >> 1) ^ mask;
            }
        }
        !crc
    }
    fn process_data(&self, file_name: &str, file_data: &str) -> Vec<u8>{
        let file_name_bytes = file_name.as_bytes();
        let file_data_bytes = file_data.as_bytes();
        let file_name_len: u32 = file_name_bytes.len() as u32;
        let file_data_len: u32 = file_data_bytes.len() as u32;
        let mut payload: Vec<u8> = Vec::new();

        // [4B Magic]
        payload.extend_from_slice(&self.magic_name.to_be_bytes());

        // [4B 檔案名稱長度]
        payload.extend_from_slice(&file_name_len.to_be_bytes());

        // [4B 資料長度]
        payload.extend_from_slice(&file_data_len.to_be_bytes());
        // [4B CRC32(檔案名稱長度+資料長度)]
        // 這裡對「長度欄位的 8 bytes」做 CRC32： [name_len(4B)][data_len(4B)]
        let mut len_block = [0u8; 8];
        len_block[0..4].copy_from_slice(&file_name_len.to_be_bytes());
        len_block[4..8].copy_from_slice(&file_data_len.to_be_bytes());
        let len_crc = self.crc32(&len_block);
        payload.extend_from_slice(&len_crc.to_be_bytes());
        // [檔案名稱資料]
        payload.extend_from_slice(file_name_bytes);
        // [4B CRC32(檔案名稱資料)]
        let name_crc = self.crc32(file_name_bytes);
        payload.extend_from_slice(&name_crc.to_be_bytes());
        // [真實資料]
        payload.extend_from_slice(file_data_bytes);
        // [4B CRC32(真實資料)]
        let data_crc = self.crc32(file_data_bytes);
        payload.extend_from_slice(&data_crc.to_be_bytes());
        payload
    }


    pub fn show_usage_cluster(&self) {
        println!("檢查目前檔案佔用了那些區塊⬇");
        let usage = self.check_usage();
        for i in usage {
            if i {
                print!("+");
            } else {
                print!("-");
            }
        }
        println!("");
    }
    pub fn show_usage_capacity(&self){
        let usage = self.check_usage();
        let mut usage_capacity: u32 = 0;

        for i in usage {
            if i {
                usage_capacity += CLUSTER_SIZE as u32;
            } 
        }
        if self.flash_size >= 1024 * 1024 * 1024 {
            println!("總容量 --> {:?} gb", (self.flash_size as f32)/(1024 as f32)/(1024 as f32)/(1024 as f32));
        } else if self.flash_size >= 1024 * 1024 {
            println!("總容量 --> {:?} mb", (self.flash_size as f32)/(1024 as f32)/(1024 as f32));
        } else if self.flash_size >= 1024 {
            println!("總容量 --> {:?} kb", (self.flash_size as f32)/(1024 as f32));
        } else {
            println!("總容量 --> {:?} bite", self.flash_size);
        }

        if usage_capacity >= 1024 * 1024 * 1024 {
            println!("使用容量 --> {:?} gb", (usage_capacity as f32)/(1024 as f32)/(1024 as f32)/(1024 as f32));
        } else if usage_capacity >= 1024 * 1024 {
            println!("使用容量 --> {:?} mb", (usage_capacity as f32)/(1024 as f32)/(1024 as f32));
        } else if usage_capacity >= 1024 {
            println!("使用容量 --> {:?} kb", (usage_capacity as f32)/(1024 as f32));
        } else {
            println!("使用容量 --> {:?} bite", usage_capacity);
        }
    }
    fn check_usage(&self) -> Vec<bool>{

        let mut usage = alloc::vec![false; self.cluster_max_quantity as usize];


        for i in 0..self.cluster_max_quantity{
            if self.check_used(i) {
                usage[i as usize] = true;
            }
        }
        usage
    }
    fn print_hex(&self,label: &str, bytes: &[u8]) {
        print!("{label} (len={}): ", bytes.len());
        for (i, b) in bytes.iter().enumerate() {
            if i != 0 {
                print!(" ");
            }
            print!("{:02X}", b);
        }
        println!("");
    }

    pub fn write(&mut self, file_name: &str, file_data: &str){
        let payload = self.process_data(file_name, file_data);
        //self.print_hex("payload", &payload);
        let cluster_vec = self.check_file_name_exist(file_name); // 檢查目前使用file_name名稱的檔案佔用了那些區塊
        self.save_cluster(payload);  //寫入新檔案
        self.delete_cluster(cluster_vec);  //將重複的舊檔案刪除
    }


    fn delete_cluster (&mut self, cluster_vec: Vec<bool>) {
        // 刪除指定區塊的內容
        let mut delete_addr: u32 = 0x000000;  //避免編譯錯誤的預設值，但原則上應用不到個預設值
        let mut flash = FlashStorage::new();
        for i in 0..self.cluster_max_quantity{
            if cluster_vec[i as usize] {
                self.cluster[0..CLUSTER_SIZE].fill(0xFF);
                let delete_addr = self.flash_addr + CLUSTER_SIZE as u32 * i as u32;
                flash.write(delete_addr, &self.cluster).unwrap();
            }
        }
    }
    pub fn delete_all_data (&mut self) {
        // 刪除指定區塊的內容
        let mut delete_addr: u32 = 0x000000;  //避免編譯錯誤的預設值，但原則上應用不到個預設值
        let mut flash = FlashStorage::new();
        for i in 0..self.cluster_max_quantity{
            self.cluster[0..CLUSTER_SIZE].fill(0xFF);
            let delete_addr = self.flash_addr + CLUSTER_SIZE as u32 * i as u32;
            flash.write(delete_addr, &self.cluster).unwrap();
        }
    }

    pub fn delete(&mut self, file_name: &str) {
        // 刪除檔名的內容
        let mut delete_addr: u32 = 0x000000; //避免編譯錯誤的預設值，但原則上應用不到個預設值
        let mut flash = FlashStorage::new();
        let cluster_vec = self.check_file_name_exist(file_name);
        for i in cluster_vec {
            if i {
                self.cluster[0..CLUSTER_SIZE].fill(0xFF);
                let delete_addr = self.flash_addr + CLUSTER_SIZE as u32 * i as u32;
                flash.write(delete_addr, &self.cluster).unwrap();
            }
        }
    }

    pub fn show_file_name_exist(&self, file_name: &str){
        println!("檢查目前使用【{file_name}】的檔案佔用了那些區塊⬇");
        let cluster_vec = self.check_file_name_exist(file_name);
        for i in cluster_vec {
            if i {
                print!("+");
            } else {
                print!("-");
            }
        }
        println!("");
    }
    fn addr_to_cluster_index(&self, addr: u32) -> Option<usize> {
        if addr < self.flash_addr {
            return None;
        }

        let offset = addr - self.flash_addr;
        if offset % CLUSTER_SIZE as u32 != 0 {
            return None;
        }

        let idx = (offset / CLUSTER_SIZE as u32) as usize;
        if idx >= self.cluster_max_quantity as usize {
            return None;
        }

        Some(idx)
    }

    fn check_file_name_exist(&self, file_name: &str) -> Vec<bool> {
        // 檢查目前使用 file_name 名稱的檔案佔用了那些區塊
        let mut cluster_vec = alloc::vec![false; self.cluster_max_quantity as usize];

        for i in 0..self.cluster_max_quantity {
            if !self.check_used(i) {
                continue;
            }

            let Some(data_start) = self.magic_start_index_in_cluster(i) else {
                continue;
            };

            let mut cur_addr = self.flash_addr + CLUSTER_SIZE as u32 * i as u32;
            let mut bytes = [0u8; CLUSTER_SIZE];
            let mut flash = FlashStorage::new();
            flash.read(cur_addr, &mut bytes).unwrap();

            if bytes[data_start..data_start + 4] != self.magic_name.to_be_bytes() {
                continue;
            }

            let file_name_len_bytes = &bytes[data_start + 4..data_start + 8];
            let file_name_len = u32::from_be_bytes(file_name_len_bytes.try_into().unwrap()) as usize;


            let name_start = data_start + 16;
            let name_end = name_start + file_name_len;
            if name_end > CLUSTER_SIZE {
                continue;
            }

            let data_path = match str::from_utf8(&bytes[name_start..name_end]) {
                Ok(s) => s,
                Err(_) => continue,
            };

            if data_path != file_name {
                continue;
            }

            loop {
                if let Some(idx) = self.addr_to_cluster_index(cur_addr) {
                    cluster_vec[idx] = true;
                }

                let continued_a_bytes = &bytes[CLUSTER_SIZE - 8..CLUSTER_SIZE - 4];
                let continued_a = u32::from_be_bytes(continued_a_bytes.try_into().unwrap());
                if continued_a != self.continued_A {
                    break;
                }

                let next_addr_bytes = &bytes[CLUSTER_SIZE - 4..CLUSTER_SIZE];
                let next_addr = u32::from_be_bytes(next_addr_bytes.try_into().unwrap());

                let Some(_) = self.addr_to_cluster_index(next_addr) else {
                    break;
                };

                cur_addr = next_addr;
                flash.read(cur_addr, &mut bytes).unwrap();
            }
        }

        cluster_vec
    }


    pub fn read(&mut self, file_name: &str) -> String {
        let mut usage = alloc::vec![false; self.cluster_max_quantity as usize];
        let mut file_data = String::new();
        for i in 0..self.cluster_max_quantity {
            if self.check_used(i) {
                let pos = self.magic_start_index_in_cluster(i);

                match pos {
                    Some(data_start) => {
                        let mut next_addr = self.flash_addr + CLUSTER_SIZE as u32 * i as u32;
                        let mut bytes = [0u8; CLUSTER_SIZE];
                        let mut flash = FlashStorage::new();
                        flash.read(next_addr, &mut bytes).unwrap();
                        if &bytes[data_start..data_start + 4] == self.magic_name.to_be_bytes() {
                            let file_name_bytes = &bytes[data_start + 4..data_start + 8];
                            let file_name_len = u32::from_be_bytes(file_name_bytes.try_into().unwrap());

                            let name_len = file_name_len as usize;
                            let data_path_bytes = &bytes[data_start + 16..data_start + 16 + name_len];
                            let name_end = data_start + 16 + name_len;
                            let data_path = match str::from_utf8(data_path_bytes) {
                                Ok(s) => s,
                                Err(_) => {
                                    println!("filename is not valid UTF-8");
                                    ""
                                }
                            };

                            if data_path == file_name {
                                // 先假設檔名都在第1區塊內，未來要檢查這部分
                                let file_data_len_bytes = &bytes[data_start + 8..data_start + 12];  // 數據資料的長度
                                let mut file_data_len = u32::from_be_bytes(file_data_len_bytes.try_into().unwrap());// 數據資料的長度
                                let mut file_data_bytes: Vec<u8> = Vec::new();
                                let mut data_start = name_end + 4;
                                loop {
                                    let read_continued_A_bytes = &bytes[CLUSTER_SIZE - 8..CLUSTER_SIZE - 4];
                                    let read_continued_A = u32::from_be_bytes(read_continued_A_bytes.try_into().unwrap());

                                    if read_continued_A == self.continued_A {

                                        file_data_bytes.extend_from_slice(&bytes[data_start..CLUSTER_SIZE - 8]);

                                        let next_addr_bytes = &bytes[CLUSTER_SIZE - 4..CLUSTER_SIZE];
                                        next_addr = u32::from_be_bytes(next_addr_bytes.try_into().unwrap());

                                        flash.read(next_addr, &mut bytes).unwrap();

                                        file_data_len -= (CLUSTER_SIZE as u32 - 8 - data_start as u32);

                                        data_start = 8;

                                    } else {
                                        file_data_bytes.extend_from_slice(&bytes[data_start..data_start + file_data_len as usize]);
                                        break;
                                    }
                                }
                                file_data = match String::from_utf8(file_data_bytes) {
                                    Ok(s) => s,
                                    Err(_) => {
                                        println!("data is not valid UTF-8");
                                        String::new()
                                    }
                                };
                                break;
                            }
                        }
                    }
                    None => {
                        file_data.clear();  //正常情況不應該輸出此值
                    }
                }
            }
        }
        file_data
    }



    fn magic_start_index_in_cluster(&self, cluster_number: u32) -> Option<usize> {
        // 返回該 cluster 內 magic 或 magic_next 的起始 index
        if cluster_number >= self.cluster_max_quantity {
            return None;
        }

        let addr = self.flash_addr + CLUSTER_SIZE as u32 * cluster_number;
        let mut bytes = [0u8; CLUSTER_SIZE];

        let mut flash = FlashStorage::new();
        flash.read(addr, &mut bytes).ok()?;

        self.find_magic_after_leading_ff(&bytes)
    }







    fn find_magic_after_leading_ff(&self, data: &[u8; CLUSTER_SIZE]) -> Option<usize> {
        // 跳過前面不確定數量的 0xFF
        let mut i = 0usize;
        while i < data.len() && data[i] == 0xFF {
                i += 1;
            }
            if i + 4 > data.len() {
                return None;
            }
            if data[i..i + 4] == self.magic_name.to_be_bytes() || data[i..i + 4] == self.continued_B.to_be_bytes(){
                Some(i)
            } else {
                None
        }
    }

    
    fn check_used(&self, cluster_number: u32) -> bool {
        let next_addr = self.flash_addr + CLUSTER_SIZE as u32 * cluster_number;
        let mut bytes = [0u8; CLUSTER_SIZE];
        let mut flash = FlashStorage::new();
        flash.read(next_addr, &mut bytes).unwrap();
        let pos = self.find_magic_after_leading_ff(&bytes);

        match pos {
            Some(i) => {
                true  //那個區塊有資料
            }
            None => {
                false  //那個區塊沒有資料
            }
        }
    }
    fn next_cluster(&self) -> u32 {
        let mut N = 0;
        loop {
            if self.check_used(N) {
                N += 1;
            } else {
               break;
            }
        }
        N
    }
    fn next_next_cluster(&self) -> u32 {
        let mut N = 0;
        let mut two = false;
        loop {
            if self.check_used(N) {
                N += 1;
            } else if two && self.check_used(N) == false {
                break;
            } else {
                two = true;
                N += 1;
            }
        }
        N
    }
    fn need_quantity(&self, payload: &Vec<u8>) -> u32 {
        let mut data_let: isize = payload.len() as isize;
        let mut n = 0;
        loop {
            if n == 0 {
                data_let -= CONCRETE_CLUSTER_SIZE as isize;
            } else {
                data_let -= CONCRETE_CLUSTER_SIZE as isize + 8;
            }
            n += 1;

            if data_let <= 0 {
                break;
            }
        
        }
        n
    }

    fn save_cluster(&mut self, payload: Vec<u8>){
        // 儲存邏輯 [4byte（Magic Number = 0x01311AAB）][4byte（檔案名稱長度）][4byte（資料長度）][4bite(檔案名稱長度+資料長度CRC32)][檔案名稱資料][4bite(檔案名稱資料CRC32)][真實資料][4bite(真實資料CRC32)]
        // 如果需要分區的話，在每個要分區的尾端加上[4byte（未結束標示A）（continued_A = 0x01311AAC）][4byte（下一個區塊的地址）]
        // 接續的cluster的前端要加上[4byte（未結束標示B）（continued_B = 0x01311AAD）][4byte（上一個區塊的地址）]
  

        let require_clusters_n = self.need_quantity(&payload);   //確認payload需要的clusters數量

    

        if require_clusters_n == 1 {
            self.cluster[0..CLUSTER_SIZE].fill(0xFF);
            let not_data_let = CONCRETE_CLUSTER_SIZE -payload.len();
        
            let mut rng = Rng::new();
            let r: u32 = rng.random();
            let front_not_data_let: usize = (r % (not_data_let as u32 + 1)) as usize; // 前面要填充0xFF的數量

            let back_not_data_let = not_data_let - front_not_data_let;  // 後面要填充0xFF的數量

            self.cluster[0..front_not_data_let].fill(0xFF);
            self.cluster[front_not_data_let..front_not_data_let + payload.len()].copy_from_slice(&payload);
            self.cluster[front_not_data_let + payload.len()..front_not_data_let + payload.len() as usize + back_not_data_let].fill(0xFF);

            let next_cluster_n = self.next_cluster();
            let next_addr = self.flash_addr + CLUSTER_SIZE as u32 * next_cluster_n;
            let mut flash = FlashStorage::new();
            flash.write(next_addr, &self.cluster).unwrap();

        } else {
            let mut remaining = 0 as usize;
            let mut last_addr: u32 = 0x000000;  //避免編譯錯誤的預設值，但原則上應用不到個預設值
            let mut next_addr: u32 = 0x000000;  //避免編譯錯誤的預設值，但原則上應用不到個預設值
            let mut next_next_addr: u32 = 0x000000;  //避免編譯錯誤的預設值，但原則上應用不到個預設值
            //println!("需要幾個clusters --> {:?}", require_clusters_n);
            for i in 0..require_clusters_n {
                self.cluster[0..CLUSTER_SIZE].fill(0xFF);

                if i == 0 {
                    let next_cluster_n = self.next_cluster();
                    next_addr = self.flash_addr + CLUSTER_SIZE as u32 * next_cluster_n;
                    self.cluster[0..CONCRETE_CLUSTER_SIZE].copy_from_slice(&payload[remaining..CONCRETE_CLUSTER_SIZE]);
                    remaining += CONCRETE_CLUSTER_SIZE;
                } else if i == require_clusters_n - 1 {
                    let next_cluster_n = self.next_cluster();
                    next_addr = self.flash_addr + CLUSTER_SIZE as u32 * next_cluster_n;
                    self.cluster[0..4].copy_from_slice(&self.continued_B.to_be_bytes());
                    self.cluster[4..8].copy_from_slice(&last_addr.to_be_bytes());
                    self.cluster[8..8 + payload.len() - remaining].copy_from_slice(&payload[remaining..payload.len()]);
                } else {
                    let next_cluster_n = self.next_cluster();
                    next_addr = self.flash_addr + CLUSTER_SIZE as u32 * next_cluster_n;
                    self.cluster[0..4].copy_from_slice(&self.continued_B.to_be_bytes());
                    self.cluster[4..8].copy_from_slice(&last_addr.to_be_bytes());
                    self.cluster[8..CONCRETE_CLUSTER_SIZE].copy_from_slice(&payload[remaining..remaining + CONCRETE_CLUSTER_SIZE - 8]);
                    remaining += CONCRETE_CLUSTER_SIZE - 8;
                }
                if i != require_clusters_n-1{
                    let next_next_cluster_n = self.next_next_cluster();
                    next_next_addr = self.flash_addr + CLUSTER_SIZE as u32 * next_next_cluster_n;
                    self.cluster[CLUSTER_SIZE - 8..CLUSTER_SIZE - 4].copy_from_slice(&self.continued_A.to_be_bytes());
                    self.cluster[CLUSTER_SIZE - 4..CLUSTER_SIZE].copy_from_slice(&next_next_addr.to_be_bytes());
                }

                last_addr = next_addr;

                let mut flash = FlashStorage::new();
                flash.write(next_addr, &self.cluster).unwrap();
                
            }
        }
    }


}
