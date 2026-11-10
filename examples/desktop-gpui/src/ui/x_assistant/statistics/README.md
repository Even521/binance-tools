# X Assistant Daily Statistics

This directory is a separate module boundary for the compact, single-page automatic-reply statistics view.

## Planned structure

- `mod.rs`: module boundary and internal module declarations.
- `model.rs`: typed daily report, reply outcomes, stages, and chart data.
- `log_parser.rs`: read-only parsing of the current Beijing-time daily X Assistant log.
- `aggregation.rs`: deduplicate log events into reply rounds and calculate totals, stage failures, hourly trends, and durations.
- `render.rs`: compact GPUI dashboard rendering. All cards and charts must fit in the existing X Assistant page viewport without vertical scrolling.

## Data flow

```text
logs/x-assistant-YYYY-MM-DD.log
             |
             v
        log_parser
             |
             v
        aggregation
             |
             v
       DailyReplyStats
             |
             v
          render
             |
             v
  one compact, non-scrolling page
```

## Counting rules

- Only include the selected Beijing calendar day; the default is today.
- Count one automation round once, even when that round emits multiple log lines.
- Keep success, explicit failure, uncertain send status, and normal skip separate.
- Calculate the explicit failure rate from failed / (success + failed). Do not silently include uncertain or skipped rounds in that denominator.
- Attribute a round to its actual failing stage; do not count multiple messages from the same failed round as multiple failures.
- Calculate elapsed time only when both start and end events are present.

## Scope of this scaffold

This commit only establishes the directory and module boundaries for review. It does not connect a new page, change the existing X Assistant UI, or alter automation and log-writing behavior. The parser, aggregation, and rendering implementation will be added after the architecture is reviewed.
