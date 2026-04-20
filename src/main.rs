use std::time::Instant;

fn fibonacci(n: u32) -> u32 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

fn main() {
    // 1. 记录进程启动后的第一时间
    let process_start = Instant::now();

    let n = 45;
    let result_start = Instant::now();
    let result = fibonacci(n);
    let duration = result_start.elapsed();

    println!("Result: {}", result);
    println!("Execution_Time: {:?}", duration); // 纯函数计算时间
    println!("Total_Process_Time: {:?}", process_start.elapsed()); // 包含初始化开销的时间
}