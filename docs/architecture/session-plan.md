# SessionPlan 與交易日

`run-planner::SessionPlan` 是執行規劃階段唯一的 session 視窗來源。它以
`InstrumentId`、交易日、profile 及選取的 `SessionKind` 建立不可變計畫，並把
calendar/profile/window-policy 版本納入 identity。

目前內建 profile：

- TWSE 2330：regular 09:00--13:30。
- TAIFEX `TXFH6`：regular 08:45--13:45、after-hours 前一個交易日 15:00 至交易日 05:00。
- TAIFEX `CDFH6`：regular 08:45--13:45、after-hours 前一個交易日 17:25 至交易日 05:00。
- TAIFEX `CAFH6`：只允許 regular。

每個官方視窗兩側固定加上五分鐘 replay margin，採開始包含、結束不包含。跨午夜
視窗使用實際 `MatchTime` 表示，不以交易日字串拼接或製造額外市場事件。週末會由
calendar version 1 向前尋找上一個工作日；假日表擴充時必須升版並重算 identity。
