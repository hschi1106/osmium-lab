"""Independent reference for osmium-md's `dump` example.

Writes one feed day as text from the marketdata parquet files with polars, in the format of
`cargo run -p osmium-md --example dump`, so the two outputs can be compared byte for byte.
The join is the one of marketdata's `md.read.replay`: `books` left-joined with `trades` on
`seq`, plus the `trades` rows that have no `books` row. The rules of
docs/reference/marketdata.md are implemented here a second time, on purpose.

    python reference_dump.py <md-root> <date> <feed> <out> [symbol...]

Needs polars; marketdata's own environment has it.
"""
import sys

import polars as pl

root, date, feed, out, *symbols = sys.argv[1:]
venue = feed.split("_")[0]
year = date[:4]

books = pl.scan_parquet(f"{root}/books/{year}/{date}/{feed}.parquet")
trades = pl.scan_parquet(f"{root}/trades/{year}/{date}/{feed}.parquet")

messages = pl.concat([
    books.join(trades.select("seq", "price", "qty", "cum_qty"), on="seq", how="left")
    .with_columns(_intermediate=pl.lit(False)),
    trades.join(books.select("seq"), on="seq", how="anti").with_columns(_intermediate=pl.lit(True)),
], how="diagonal_relaxed").sort("seq")
# Observed time spans the whole feed, so it is computed before any symbol is dropped.
messages = messages.with_columns(_observed=pl.col("exch_ts").cum_max())
if symbols:
    messages = messages.filter(pl.col("symbol").is_in(symbols))

if feed.endswith("F23"):
    messages = messages.with_columns(_lot=pl.lit(1, dtype=pl.Int64))
else:
    lots = pl.scan_parquet(f"{root}/instruments/{year}/{date}/{venue}.parquet").select("symbol", "shares_per_lot")
    messages = messages.join(lots, on="symbol", how="left").sort("seq").with_columns(
        _lot=pl.col("shares_per_lot").fill_null(1000).cast(pl.Int64))


def units(column: str) -> pl.Expr:
    return (pl.col(column) * 10000).round().cast(pl.Int64)


def shares(column: str) -> pl.Expr:
    return pl.col(column) * pl.col("_lot")


def when(condition: pl.Expr, text: pl.Expr) -> pl.Expr:
    return pl.when(condition).then(text).otherwise(pl.lit(""))


def side(name: str) -> pl.Expr:
    market = pl.col(f"{name}_mkt_qty").fill_null(0)
    levels = pl.concat_str([
        pl.when(pl.col(f"{name}_px_{i}").is_not_null())
        .then(pl.format("{}*{}", units(f"{name}_px_{i}"), shares(f"{name}_qty_{i}")))
        for i in range(1, 6)
    ], separator=",", ignore_nulls=True)
    return pl.format("\t{}={}|{}", pl.lit(name), levels, market * pl.col("_lot"))


def flag(column: str) -> pl.Expr:
    return when(pl.col(column).is_not_null(), pl.format("\t{}={}", pl.lit(column), pl.col(column).cast(pl.Int8)))


book = pl.concat_str([side("bid"), side("ask")])
trade = pl.concat_str([
    when(pl.col("price").is_not_null(), pl.format("\ttrade={}*{}", units("price"), shares("qty"))),
    when(pl.col("cum_qty").is_not_null(), pl.format("\tcum_qty={}", shares("cum_qty"))),
])
matched = when(pl.col("indicative_px").is_not_null(),
               pl.format("\tmatched={}*{}", units("indicative_px"), shares("indicative_qty")))
body = (
    pl.when(pl.col("_intermediate")).then(pl.concat_str([pl.lit("trade"), trade]))
    .when(pl.col("delay_state") == 1).then(pl.lit("delay\tdirection=up"))
    .when(pl.col("delay_state") == -1).then(pl.lit("delay\tdirection=down"))
    .when(pl.col("is_indicative")).then(
        pl.concat_str([pl.lit("indicative"), book, matched, flag("delay_open"), flag("delay_close")]))
    .otherwise(pl.concat_str([pl.lit("disclosure"), book, trade]))
)
line = pl.format("{}\t{}\t{}\t{}\t{}\t{}", "seq", "symbol", pl.col("exch_ts").dt.epoch("us"),
                 pl.col("_observed").dt.epoch("us"), "phase", body)

lines = messages.select(line.alias("line")).collect()["line"].to_list()
with open(out, "w") as file:
    for text in lines:
        file.write(text)
        file.write("\n")
print(f"{date} {feed}: {len(lines)} messages")
