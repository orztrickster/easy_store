#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]


use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_println as _;
use esp_println::println;  // 啟用println
use easy_store::store::Store;


#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    println!("錯誤");
    loop {}
}

extern crate alloc;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // generator version: 1.0.0


    println!("開始執行");
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 66320);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);






    let file_name_a = "/data/系統紀錄檔.txt"; // 檔案名稱（UTF-8）
    let file_data_a = "Hello World!!!";       // 檔案資料（UTF-8）

    let file_name_b = "/data/三體.txt"; // 檔案名稱
    let file_data_b = r#"  
    《三體：地球往事》
　　作者：劉慈欣

　　正文

　　前言

　　《三體》終於能與科幻朋友們見面了，用連載的方式事先誰都沒有想到，也是無奈之舉。之前就題材問題與編輯們仔細商討過，感覺沒有什麼問題，但沒想到今年是文革三十周年這事兒，單行本一時出不了，也只能這樣了。

　　其實這本書不是文革題材的，文革內容在其中只占不到十分之一，但卻是一個漂蕩在故事中揮之不去的精神幽靈。

　　本書雖不是《球狀閃電》的續集，但可以看做那個故事所發生的世界在其後的延續，那個物理學家在故事中出現但已不重要，其他的人則永遠消失了，林雲真的死了，雖然我有時在想，如果她活下來，最後是不是這個主人公的樣子？

　　這是一個暫名為《地球往事》的系列的第一部，可以看做一個更長的故事的開始。

　　這是一個關於背叛的故事，也是一個生存與死亡的故事，有時候，比起生存還是死亡來，忠誠與背叛可能更是一個問題。

　　瘋狂與偏執，最終將在人類文明的內部異化出怎樣的力量？冷酷的星空將如何拷問心中道德？

　　作者試圖講述一部在光年尺度上重新演繹的中國現代史，講述一個文明二百次毀滅與重生的傳奇。

　　朋友們將會看到，連載的這第一期，幾乎不是科幻，但這本書並不是這一期顯示出來的這個樣子，它不是現實科幻，比《球狀閃電》更空靈，希望您能耐心地看下去，後面的故事變化會很大。

　　在以後的一段時光中，讀者朋友們將走過我在過去的一年中走過的精神歷程，坦率地說，我不知道你們將在這條黑暗詭異的迷途上看到什麼，我很不安。但科幻寫到今天，能夠與大家同行這麼長一段，也是緣份。"#;






    let mut store = Store::new(0x210000, 0x100000);

    //store.delete_all_data();

    store.write(file_name_a,file_data_a);
    println!("已存檔 --> {:?}",file_name_a);
    store.write(file_name_b,file_data_b);
    println!("已存檔 --> {:?}",file_name_b);
    store.show_usage_cluster();
    store.show_usage_capacity();

    store.show_file_name_exist("/data/系統紀錄檔.txt");
    store.show_file_name_exist("/data/三體.txt");

    let file_data = store.read("/data/系統紀錄檔.txt");
    println!("讀取檔案內容 -->\n{}", file_data);

    let file_data = store.read("/data/三體.txt");
    println!("讀取檔案內容 -->\n{}", file_data);

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}







