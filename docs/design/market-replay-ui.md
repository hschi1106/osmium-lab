# 歷史行情重播 TUI 設計

第一版的操作介面、資料邊界與播放模型。

## 命令

    osmium display --config config/m4-day-multi.yaml

第一版接受既有 M2 config_version 1 或 M3 config_version 2，且只處理一個
trading_date。執行前完成：

    plan -> sync（需要時）-> verify -> cache prepare -> display

TUI 完全離線，只依 frozen ReplayPlan 開啟 explicit universe 的 cache streams；不會
自動下載、驗證 source 或建立 cache。缺少 complete source 或 valid replay cache 時，
先執行 cache prepare。osmium replay --config 仍是既有無 UI 的離線 replay command。

## 分層

    M3 config / ReplayPlan
            |
            v
    MarketReplay session -> ratatui renderer + crossterm input

MarketReplay 負責 k-way merge、match_time clock、播放倍率、pause state 及
ReplayCore.apply_ordered；UI 只讀取 session view、繪製資料及轉送按鍵。每個 stream
只保留一個 head event，因此不需把完整交易日載入記憶體。

## 播放與操作

所有 selected symbol 共用最早 replay_start 到最晚 replay_end_exclusive 的時間軸。
切換標的不會暫停、跳時或回到該商品第一筆事件。1.0x 以歷史 match_time 的 wall-clock
差值播放，固定倍率為：

    0.1x -> 0.25x -> 0.5x -> 1.0x -> 2.0x -> 5.0x -> 10.0x -> 25.0x -> 50.0x

| 按鍵 | 行為 |
| --- | --- |
| ←／→ | 切換 selected symbol |
| Space | 暫停／繼續共用播放時鐘 |
| +／=、- | 切換固定倍率 |
| R | 重新播放並恢復 1.0x |
| Q | 離開 TUI |

## 畫面資料

價格圖是目前已套用 source-observed trade price 的簡單折線，不建立 K 線、不預看事件，
也不把 indicative auction 當成 actual trade。成交量圖以 `match_time` 的一分鐘桶加總已
套用成交事件的 observed quantity，並以柱狀圖呈現；與價格圖使用相同時間範圍，不計算
trade delta、imbalance 或 VWAP。

左下顯示 ReplayCore 目前 state 的完整五檔 snapshot；右下顯示 selected symbol 的最近
成交，最新在最上方。TradePrint 沒有可驗證的 aggressor side，所以 SIDE 顯示 —，不猜測
BUY／SELL；domain book 沒有逐筆 order count，所以只顯示 LEVEL、PRICE、QTY。

## 驗證

TUI 不提供 degraded mode；cache、ordering 或 state 驗證失敗即停止。終端離開時恢復 raw
mode 與 alternate screen。第一版以 PlaybackSpeed 單元測試、CLI parser 測試、cargo
fmt --check、cargo test 及已準備 M3 cache 的手動操作檢查驗證。
