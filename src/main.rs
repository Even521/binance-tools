//! 默认二进制入口：薄封装，具体逻辑优先放在库模块 [`crate`] 中。

fn main() {
    println!("{}", binance_tools::greeting());
}
