//! 默认二进制入口：薄封装，具体逻辑优先放在库模块 [`binance_tools`] 中。

fn main() {
    let settings = binance_tools::binance::BinanceSettings::testnet();

    println!("{}", binance_tools::greeting());
    println!("Binance environment: {}", settings.environment());
    println!("REST endpoint: {}", settings.rest_base_url());
}
