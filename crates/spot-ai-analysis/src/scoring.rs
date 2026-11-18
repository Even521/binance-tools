//! USDT 现货 1–7 天波段选币 V2：趋势、成交、风险、入场、流动性与情绪。
use crate::{AnalysisResult, DailyMaSnapshot, MarketSentiment, MarketSnapshot};
use std::{cmp::Ordering, collections::HashMap};

/// 趋势、成交、风险、入场、情绪、流动性。
pub const SCORE_WEIGHTS: [f64; 6] = [0.30, 0.20, 0.20, 0.15, 0.05, 0.10];
const STABLECOIN_BASE_ASSETS: &[&str] = &[
    "USDT","USDC","BUSD","TUSD","FDUSD","DAI","USDP","USDE","PYUSD","USDS",
    "GUSD","FRAX","LUSD","USDD","UST","USTC","USD1","USDJ","CRVUSD","EURI",
    "AEUR","RLUSD","USDG",
];
fn is_stablecoin_base_asset(asset: &str) -> bool {
    STABLECOIN_BASE_ASSETS.iter().any(|s| asset.eq_ignore_ascii_case(s))
}
fn clamp_score(value: f64) -> f64 { if value.is_finite() { value.clamp(0.0,100.0) } else { 50.0 } }
fn percentile(values: &[f64], value: f64) -> Option<f64> {
    if values.is_empty() || !value.is_finite() { return None; }
    Some(values.partition_point(|v| *v <= value) as f64 / values.len() as f64 * 100.0)
}
fn average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}
fn returns(closes: &[f64], days: usize) -> Option<f64> {
    if closes.len() <= days { return None; }
    let old = closes[closes.len()-1-days];
    (old > 0.0).then(|| (closes[closes.len()-1] / old - 1.0) * 100.0)
}
fn score_return(value: f64, scale: f64) -> f64 { clamp_score(50.0 + value * scale) }

pub fn analyze_market(products: &[MarketSnapshot], cached_ma: &[DailyMaSnapshot]) -> Vec<AnalysisResult> {
    analyze_market_with_sentiment(products, cached_ma, None)
}
pub fn analyze_market_with_sentiment(
    products: &[MarketSnapshot], cached_ma: &[DailyMaSnapshot], sentiment: Option<&MarketSentiment>,
) -> Vec<AnalysisResult> {
    let candidates = products.iter().filter(|p|
        p.quote_asset == "USDT" && p.is_trading && !p.is_etf && !is_stablecoin_base_asset(&p.base_asset)
    ).collect::<Vec<_>>();
    let mut turnovers = candidates.iter().filter_map(|p| p.quote_volume)
        .filter(|v| v.is_finite() && *v > 0.0).collect::<Vec<_>>();
    turnovers.sort_by(|a,b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let ma30 = cached_ma.iter().map(|s|(s.symbol.as_str(),s)).collect::<HashMap<_,_>>();
    let sentiment_valid = sentiment.filter(|s| s.value.is_finite() && (0.0..=100.0).contains(&s.value));

    let mut results = candidates.into_iter().map(|p| {
        let candles = &p.daily_klines;
        let closes = candles.iter().map(|k| k.close).filter(|v| v.is_finite() && *v > 0.0).collect::<Vec<_>>();
        let last = closes.last().copied().or(p.last_price);
        let r1 = returns(&closes,1);
        let r3 = returns(&closes,3);
        let r7 = returns(&closes,7);
        let ma30_price = (closes.len() >= 30).then(|| average(&closes[closes.len()-30..])).flatten();
        let ma120_price = (closes.len() >= 120).then(|| average(&closes[closes.len()-120..])).flatten();
        let mut available = [false;6];

        let trend_score = if let (Some(a),Some(b),Some(c))=(r1,r3,r7) {
            available[0]=true;
            let momentum = score_return(a,3.0)*0.20 + score_return(b,2.0)*0.35 + score_return(c,1.2)*0.45;
            let structure = match (last,ma30_price,ma120_price) {
                (Some(price),Some(m30),Some(m120)) if m30>0.0 && m120>0.0 =>
                    (if price>m30 {60.0} else {40.0})*0.35 + (if m30>m120 {60.0} else {40.0})*0.65,
                (Some(price),Some(m30),_) if m30>0.0 => if price>m30 {55.0}else{45.0},
                _ => 50.0,
            };
            clamp_score(momentum*0.7+structure*0.3)
        } else { 50.0 };

        let volume_rank = percentile(&turnovers,p.quote_volume.unwrap_or(f64::NAN));
        let recent_avg_turnover = (candles.len()>=20).then(|| average(&candles[candles.len()-20..].iter()
            .map(|k|k.quote_turnover).filter(|v|v.is_finite() && *v>0.0).collect::<Vec<_>>())).flatten();
        let relative_volume = p.quote_volume.zip(recent_avg_turnover).filter(|(_,avg)|*avg>0.0)
            .map(|(current,avg)|current/avg);
        let volume_score = match (volume_rank,relative_volume) {
            (Some(rank),Some(ratio)) => { available[1]=true; clamp_score(rank*0.55+score_return((ratio-1.0)*100.0,0.35)*0.45) },
            (Some(rank),None) => { available[1]=true; rank },
            _ => 50.0,
        };

        let atr_pct = if candles.len()>=15 {
            let start=candles.len()-14;
            let mut trs=Vec::new();
            for i in start..candles.len() {
                let prev=candles[i-1].close;
                if prev>0.0 {
                    trs.push((candles[i].high-candles[i].low)
                        .max((candles[i].high-prev).abs()).max((candles[i].low-prev).abs()));
                }
            }
            average(&trs).zip(last).filter(|(_,price)|*price>0.0).map(|(atr,price)|atr/price*100.0)
        } else { None };
        let drawdown7 = if candles.len()>=7 {
            let recent=&candles[candles.len()-7..];
            let peak=recent.iter().map(|k|k.high).fold(0.0_f64,f64::max);
            last.filter(|_|peak>0.0).map(|price|(price/peak-1.0)*100.0)
        } else { None };
        let risk_score = match (atr_pct,drawdown7) {
            (Some(atr),Some(dd)) => { available[2]=true; clamp_score(100.0-atr*7.0-dd.abs()*2.0) },
            (Some(atr),None) => { available[2]=true; clamp_score(100.0-atr*7.0) },
            _ => 50.0,
        };

        let entry_score = match (last,ma30_price,candles.get(candles.len().saturating_sub(20)..)) {
            (Some(price),Some(ma),Some(window)) if ma>0.0 && !window.is_empty() => {
                available[3]=true;
                let distance=(price/ma-1.0)*100.0;
                let high=window.iter().map(|k|k.high).fold(0.0_f64,f64::max);
                let high_position=if high>0.0 {(price/high*100.0).clamp(0.0,100.0)} else {50.0};
                clamp_score(70.0-distance.abs()*5.0 + (100.0-high_position)*0.25)
            },
            _=>50.0,
        };

        let sentiment_score=sentiment_valid.map(|s|{available[4]=true;s.value}).unwrap_or(50.0);
        let liquidity_score=match (volume_rank,p.quote_volume) {
            (Some(rank),Some(turnover)) if turnover>0.0 => {available[5]=true; clamp_score(rank*0.8+ (turnover.log10().max(0.0)*2.0).min(20.0))},
            _=>50.0,
        };
        let weights=available.iter().enumerate().filter(|(_,ok)|**ok).map(|(i,_)|SCORE_WEIGHTS[i]).sum::<f64>();
        let total=if weights>0.0 {
            let scores=[trend_score,volume_score,risk_score,entry_score,sentiment_score,liquidity_score];
            scores.iter().enumerate().filter(|(i,_)|available[*i])
                .map(|(i,s)|s*SCORE_WEIGHTS[i]/weights).sum()
        } else {50.0};
        let completeness=available.iter().enumerate().filter(|(_,ok)|**ok).map(|(i,_)|SCORE_WEIGHTS[i]).sum::<f64>()*100.0;
        let change=p.price_change_percent.unwrap_or(0.0);
        let label=if trend_score>=70.0 {"趋势偏强"} else if trend_score>=55.0 {"温和走强"}
            else if trend_score<=30.0 {"趋势偏弱"} else if trend_score<=45.0 {"温和走弱"} else {"区间震荡"}.to_string();
        let basis=format!(
            "1/3/7日收益={}/{}/{}；MA30={}；MA120={}；ATR14={}；7日回撤={}；20日成交额均值={}；有效因子权重={:.0}%。24h涨跌={:+.2}%。",
            fmt(r1),fmt(r3),fmt(r7),fmt(ma30_price),fmt(ma120_price),fmt(atr_pct),fmt(drawdown7),
            fmt(recent_avg_turnover),completeness,change
        );
        AnalysisResult{
            symbol:p.symbol.clone(),trend_score,volume_score,technical_score:entry_score,
            risk_score,sentiment_score,sentiment_value:sentiment_valid.map(|s|s.value).unwrap_or(50.0),
            sentiment_classification:sentiment_valid.map(|s|s.classification.clone()).unwrap_or_else(||"数据暂不可用".into()),
            sentiment_update_time:sentiment_valid.map(|s|s.update_time.clone()).unwrap_or_default(),
            liquidity_score,total_score:total,data_completeness:completeness,trend_label:label,basis
        }
    }).collect::<Vec<_>>();
    results.sort_by(|a,b|b.total_score.partial_cmp(&a.total_score).unwrap_or(Ordering::Equal));
    results
}
fn fmt(v:Option<f64>)->String{v.map(|x|format!("{x:.2}")).unwrap_or_else(||"缺失".into())}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(symbol:&str, change:f64, turnover:f64)->MarketSnapshot {
        MarketSnapshot{symbol:symbol.into(),base_asset:symbol.into(),asset_name:symbol.into(),
            quote_asset:"USDT".into(),is_trading:true,is_etf:false,price_change_percent:Some(change),
            quote_volume:Some(turnover),last_price:Some(100.0),high_price:Some(105.0),low_price:Some(95.0),
            volume:Some(1000.0),daily_klines:vec![]}
    }
    #[test] fn excludes_stablecoins_and_nontrading(){
        let mut inactive=sample("OFF",20.0,9000.0);inactive.is_trading=false;
        let rows=analyze_market(&[sample("BTC",2.0,5000.0),sample("USDC",1.0,9999.0),inactive],&[]);
        assert_eq!(rows.len(),1);assert_eq!(rows[0].symbol,"BTC");
    }
    #[test] fn weights_sum_to_one(){assert!((SCORE_WEIGHTS.iter().sum::<f64>()-1.0).abs()<f64::EPSILON);}
    #[test] fn missing_history_reduces_completeness_and_renormalizes(){
        let row=analyze_market(&[sample("BTC",1.0,1000.0)],&[]).remove(0);
        assert!(row.data_completeness<100.0);
        assert!(row.total_score.is_finite());
    }
    #[test] fn daily_history_enables_trend_risk_and_entry(){
        let mut p=sample("BTC",1.0,1000.0);
        p.daily_klines=(0..121).map(|i|{
            let close=80.0+i as f64*0.2;
            crate::DailyKlineSnapshot{high:close+1.0,low:close-1.0,close,quote_turnover:10000.0}
        }).collect();
        let row=analyze_market(&[p],&[]).remove(0);
        assert!(row.data_completeness>=65.0);
        assert!(row.trend_score>50.0);
    }
}
