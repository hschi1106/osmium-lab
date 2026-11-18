# 台灣證券現股帳戶會計規則研究（for exact backtest ledger）

> 狀態：完成（章節依撰寫順序：§0 資料、§2 證交稅、§1 手續費、§3 交割、§4 當沖、§5 公司行動、§6 成本、§7 金額、§8 參數、§9 未確認）。研究日期：2026-11-18。
> 信心等級：**official**（官方法規／交易所公告）、**data-confirmed**（marketdata 資料驗證）、**unconfirmed**（未能以官方來源確認）、**broker-specific**（券商自訂 → 必須參數化）。

## 0. marketdata 中實際存在的商品類型（data-confirmed）

查詢方式：`~/marketdata/.venv/bin/python -I` + polars，read-only 掃 `/srv/md/v1/instruments/*/*/*.parquet`、`daily/202[2-6].parquet`。

- `instruments` 日期範圍：**2022-01-03 → 2026-10-16**（1,160 個交易日）。`source`：TWSE `MI_INDEX`／TPEx `TPEX_DAILY` 全期間；`F1`（交易所 Format 1，含 `f1_*` 欄）只有 **2024-04-01 → 2026-10-12**。`daily` 到 2026-11-17。
- `instrument_type` 實際值（2022–2026，distinct symbols／rows）：

| venue | type | symbols | rows |
|---|---|---|---|
| TWSE | STOCK | 1,107 | 1,173,819 |
| TWSE | ETF | 220 | 189,769 |
| TWSE | ETN | 24 | 19,693 |
| TWSE | REIT | 7（01001T…01010T） | 7,422 |
| TWSE | TDR | 5（9103/9105/9110/9136/9188） | 4,676 |
| TWSE | OTHER | 236 | 140,297 |
| TWSE | WARRANT_C／WARRANT_P | 60,166／18,119 | 18.4M／2.3M |
| TPEX | STOCK | 930 | 962,699 |
| TPEX | ETF | 128 | 109,199 |
| TPEX | ETN | 16 | 9,704 |
| TPEX | OTHER | 677 | 212,735 |
| TPEX | WARRANT_C／WARRANT_P | 40,000／3,754 | 6.0M／0.44M |

**`OTHER` 是混合袋，稅率不同，ledger 不能只看 `instrument_type`**（data-confirmed）：
- 特別股：`2881A 富邦特`、`2887E`、`2887Z1`、`1101B 台泥乙特`、`8349A 恒耀甲特`…（4–5 碼 + 英文字尾，名稱含「特」）→ 股票，0.3%。
- 6 碼 `006xxx`／`0098xx`／`009xxx` ETF（`006208 富邦台50`、`0098xx` 系列，2025 年起新上市的 6 碼 ETF 全部落在 OTHER）→ ETF，0.1%。
- ETN 帶字尾：`02001L 富邦蘋果正二N`、`02001R`、`02001S`、`02001B 統一美國政府債N`、`02002L`、`02003L` → ETN。
- 6 碼 `91xxxx` TDR（`910861 神州-DR`、`912000 晨訊科-DR`）→ TDR。
- TPEx 5 碼（`華友聯一`、`宏遠證一`…）＝**轉換公司債**；6 碼 `xxxxE1`（`遠東新E1永`）＝**交換公司債** → 公司債，證交稅停徵（見 §2）。
- `A0xxxx`／`A1xxxx`／`A9xxxx`（TWSE F1）＝**中央登錄公債**（TWSE 資訊傳輸作業手冊格式一 3.1.3(4)、3.1.8 說明「中央登錄公債」）→ 政府債券，不課證交稅。
- `ETF` 依代號字尾：無字尾 129、`A` 主動式股票 ETF 33、`B` 債券 ETF 121、`C` 2、`D` 主動式債券 ETF 8、`K` 外幣計價／雙幣 5、`L` 槓桿 21、`R` 反向 20、`T` 平衡／多資產 3、`U` 期貨型（商品）6。
- `STOCK` 內 `f1_security_type`：`RR` 外國企業（-KY）、`BS` 證券股、`FB` 銀行股、null 一般本國（TWSE 資訊傳輸作業手冊附錄四）。
- `shares_per_lot`：絕大多數 1000；少數 100（F1 中 TWSE 1,845 列、TPEx 423 列）→ ledger 不可寫死 1000。
- `f1_non10_par = true`：27 檔 STOCK（非 10 元面額，例 `3093`、`6415`）→ 股票股利／減資換算需面額。

> 建議：ledger 用一個 `tax_class` 函式（輸入 symbol、venue、instrument_type、name、date），不要直接用 `instrument_type` 查稅率；`tax_class ∈ {STOCK, ETF_EQUITY, ETF_BOND_EXEMPT, ETF_BOND_TAXED, ETN, WARRANT, TDR, REIT, CORP_BOND, GOV_BOND}`。

---

## 2. 證券交易稅（證交稅）

### 2.1 法源（official）
來源：證券交易稅條例，財政部法規資料庫 <https://law-out.mof.gov.tw/LawContent.aspx?id=FL006079>（修正日期 民國114-01-02，2026-11-18 擷取）；沿革 <https://law-out.mof.gov.tw/LawContentSource.aspx?id=FL006079>。

- 第 1 條：各級政府發行之債券不課。
- 第 2 條：**向出賣人**按**每次交易成交價格**課徵：(一) 公司發行之股票及表明股票權利之證書或憑證 **3‰**；(二) 公司債及其他經政府核准之有價證券 **1‰**。
- 第 2-1 條：公司債、金融債券 99-01-01 至 **115-12-31（2026-12-31）停徵**；以債券為主要投資標的之上市櫃「指數股票型基金」受益憑證 106-01-01 至 **115-12-31 停徵**，**但槓桿型及反向型債券 ETF 不適用**。
- 第 2-2 條（當沖）：同一證券商受託買賣，同一帳戶於同一營業日現款買進與現券賣出同種類同數量之**上市或上櫃股票**，出賣時按 **1.5‰**；期間 106-04-28（2017-04-28）起至 **116-12-31（2027-12-31）**（自營商自 107-04-28 起）。
- 第 3 條：代徵人於**每次買賣交割之當日**代徵（即 T+2 交割日扣繳）。

### 2.2 當沖減半的時間線（official：沿革頁列出 106-04-26 增訂第2-2條、107-04-27、110-12-29、114-01-02 修正第2-2條）
| 期間 | 稅率 | 依據 |
|---|---|---|
| 2017-04-28 起 | 1.5‰（當沖賣出） | 106-04-26 增訂第 2-2 條 |
| 延長至 2021-12-31 | 1.5‰ | 107-04-27 修正（unconfirmed：中間到期日未逐版核對） |
| 延長至 2024-12-31 | 1.5‰ | 110-12-29 修正；TWSE 英文交易制度頁仍寫「0.15% from April 28, 2017 to December 31, 2024」<https://www.twse.com.tw/en/products/system/trading.html> |
| 延長至 **2027-12-31** | 1.5‰ | 114-01-02 公布修正（立法院 2024-12-31 三讀）；現行條文即此版。新聞：財政部表示與 113-12-31 無縫接軌（unconfirmed：未取得財政部新聞稿原文，但現行條文已確認） |

→ **資料範圍 2022-01-03 起至今，當沖股票賣出一律 1.5‰，無中斷**（official）。2028-01-01 以後若未延長回到 3‰ → 參數 `daytrade_reduced_rate_until = 2027-12-31`。

### 2.3 稅率表（依商品；資料範圍 2022–2026）
| tax_class | 稅率（賣方） | 依據／信心 |
|---|---|---|
| 普通股、特別股、KY 股、創新板 | 3‰；**當沖 1.5‰** | 條例 §2(1)、§2-2（official） |
| ETF（股票型、主動式股票 A、槓桿 L、反向 R、期貨型 U、平衡 T、外幣 K） | 1‰；**當沖仍是 1‰**（§2-2 只適用「股票」） | TWSE 英文交易制度頁「Beneficiary Certificates / ETF / Warrants / TDRs / Company Warrants: 0.1%」（official）；當沖不減半為法條推論（official 文義） |
| 被動式債券 ETF（字尾 B，非槓桿反向） | **0（停徵 2017-01-01 → 2026-12-31）** | 條例 §2-1（official） |
| 槓桿／反向債券 ETF | 1‰ | 條例 §2-1 但書（official）；資料中目前未見此類代號（字尾 B 皆為一般債券 ETF；需逐檔確認） |
| **主動式債券 ETF（字尾 D）** | **1‰（現行未停徵）**；行政院 2026-10-01 通過草案擬自 2027-01-01 起納入停徵並延至 2036-12-31 | 財政部新聞稿 115.10.1「行政院院會通過證券交易稅條例第2條之1修正草案」<https://www.ey.gov.tw/File/4690506087663D7D?A=C>（official；草案尚待立法院，**unconfirmed 是否通過**） |
| ETN | 1‰ | 財政部稅率一覽未取得；TWSE 投資指南（搜尋摘要）列「指數投資證券 ETN 賣出 0.1%」（unconfirmed：未取得頁面原文） |
| 認購／認售權證 | 1‰ | TWSE 英文交易制度頁（official） |
| TDR | 1‰ | TWSE 英文交易制度頁「TDRs 0.1%」（official）；當沖亦不減半（TDR 非「上市櫃股票」，unconfirmed 但文義如此） |
| REIT（不動產投資信託受益證券） | **免稅** | TWSE 英文交易制度頁「Bonds / Corporate Bonds / REITs / Bond with Warrants: Tax exempted」（official 交易所說明；法源條文未取得 → 標 unconfirmed-legal-basis） |
| 轉換公司債、交換公司債、公司債 | 0（停徵至 2026-12-31） | 條例 §2-1（official） |
| 中央登錄公債 | 0（不課） | 條例 §1（official） |

### 2.4 計算基礎、粒度與進位
- 稅基＝**成交價格（成交價金＝價 × 股數）**，每次交易（條例 §2「每次交易成交價格」）。official。
- 只對**賣方**課。official。
- **進位：法規未見明文**（unconfirmed）。業界普遍做法：每筆（券商以「每筆成交／每張買賣報告書」）證交稅＝`floor(成交價金 × 稅率)`，元以下捨去。財政部對**外幣計價**證券有明文：外幣稅額四捨五入至小數 2 位、換算新臺幣後**元以下四捨五入**（財政部稅務入口網 Q&A <https://www.etax.nat.gov.tw/etwmain/tax-info/understanding/tax-q-and-a/national/securities-transaction-tax/filing/Koxx25n>，比照台財稅字第10504524600號令）。
- **粒度：broker-specific**。券商對帳單常見以「同一委託書號（同一筆委託的多筆成交合併）」或「同日同股同買賣別合併」計稅；法條字面是「每次交易」。→ 參數 `tax_aggregation ∈ {per_fill, per_order, per_day_symbol_side}`，`tax_rounding ∈ {floor, half_up}`，預設 `per_order` + `floor`（unconfirmed，需用實際對帳單校驗）。
- 當沖減稅部分：按「沖銷數量」那部分的賣出成交價金適用 1.5‰，非沖銷部分 3‰（§2-2「同種類同數量」）。賣出一筆成交可能拆成沖銷與非沖銷兩部分 → ledger 需能拆。


---

## 1. 證券商手續費（手續費）

### 1.1 法規（official）
來源：臺灣證券交易所股份有限公司營業細則 第 94 條（修正日期 民國115-11-04；TWSE 法規分享知識庫 <https://twse-regulation.twse.com.tw/TW/law/DAT0201.aspx?FLCODE=FL007304>，條文取自前一研究者下載之全文並核對條號）：
> 證券經紀商受託買賣成交後，須向委託人收取手續費，其費率由本公司申報主管機關核定之。證券經紀商收取證券交易手續費得按客戶成交金額自行訂定費率標準，另得訂定**折讓**及**每筆委託最低費用**，並於實施前透過「證券商申報單一窗口」申報本公司備查。證券經紀商於手續費收取後，得按一定期間（月、週等）結算，如有應退還或折減款項，應撥入至原客戶交割帳戶……證券經紀商所訂手續費率**逾成交金額千分之一點四二五者，應於委託前採取適當方式通知**，並留存紀錄。

TWSE 英文交易制度頁（<https://www.twse.com.tw/en/products/system/trading.html>）：「Securities firms can set up their own commission schedule… For customers charged with rate higher than the 0.1425% standard rate, the securities firm must take appropriate means to notify them」。存託憑證手續費準用上市股票（TWSE 存託憑證買賣辦法第 14 條，搜尋摘要；unconfirmed 原文）。

**結論**
- 0.1425% 是「標準費率」；**現行規章並非絕對上限**，超過須事前通知（official）。實務上幾乎所有零售券商 ≤ 0.1425%。
- 計費基礎＝**成交金額**；買賣雙邊都收（official：「受託買賣成交後」，不分買賣）。
- **最低費用以「每筆委託」為單位**由券商自訂（official 授權；金額 broker-specific）。常見：整股 20 元、零股 1 元或 20 元（各券商不同；broker-specific，非法規強制）。
- **折讓**可「即時」（成交時以折扣後費率收）或「事後」（按月／週結算退還至交割帳戶）（official 允許兩種）。
- **進位**：無法規；券商實務不同（如部分券商 floor，部分 ceil 或四捨五入）（broker-specific；搜尋所見業界文章與券商說明互相矛盾）。
- 計費粒度：法規文字「每筆委託最低費用」暗示以委託為單位計最低費；實際費額有券商「同一委託書多筆成交合併計算」、亦有「同日同股同買賣別合併」（broker-specific）。
- 當沖兩邊都收手續費；部分券商有當沖折扣（broker-specific）。

### 1.2 建議參數化
```text
FeeSchedule {
  base_rate:            Decimal = 0.001425      # 每邊
  discount:             Decimal = 1.0           # 例 0.28 = 2.8 折；可依 board(REG/ODD/FIX)、channel、daytrade 分別設定
  discount_mode:        Immediate | Rebate{period: Daily|Weekly|Monthly, pay_lag_days}
  aggregation:          PerFill | PerOrder | PerDaySymbolSide   # 先加總成交金額再算費
  rounding:             Floor | Ceil | HalfUp                    # 對「折扣前」或「折扣後」金額：rounding_stage = BeforeDiscount | AfterDiscount
  min_fee_regular:      i64 = 20                # 整股（含盤後定價），每 aggregation 單位
  min_fee_odd:          i64 = 1 (或 20)         # 盤中／盤後零股
  min_fee_applies_after_discount: bool = true
}
fee = max(min_fee, round(Σ成交金額 × base_rate × discount))   # Immediate 模式
```
Rebate 模式：成交時按 `base_rate`（含最低費）收，於結算期末把 `fee_full − fee_discounted` 以現金入帳（不影響 T+2 交割金額，影響現金時間點）。

---

## 3. 交割（Settlement）

### 3.1 T+2 款券交割（official）
- 營業細則第 57 條：「普通交割之買賣於成交日後第二營業日辦理交割。」（FL007304，修正 115-11-04）
- TWSE 集中市場交易制度介紹 <https://www.twse.com.tw/zh/products/system/trading.html>「結算交割」：
  - 買進：應付款於成交日後第二營業日**上午 10 時前**交割；應收券於 T+2 **上午 11 時後**取得。
  - 賣出：應付券於 T+2 上午 10 時前交割；應收款於 T+2 上午 11 時後取得。
  - 同時買賣：「**款按應收應付相抵後之餘額結算，券採總額結算**」。
- 證交稅於交割當日由代徵人扣繳（條例 §3），手續費一併於交割款計算（券商買賣報告書／交割單淨收付＝價金 ± 手續費 − 稅）。

### 3.2 淨額（official + 建議）
- 同一委託人同一交割日的**款項**：Σ(賣出價金 − 賣方手續費 − 證交稅) − Σ(買進價金 + 買方手續費) → 一個淨額（應收或應付）。跨股票淨額化（TWSE 用語為「款按應收應付相抵」，未限同一股票；unconfirmed 是否跨券商帳戶——不跨，交割帳戶以券商別區分）。
- **券**：總額交割，各股分別交付，不跨股相抵。
- 當沖沖銷部分只交割「差價」（當沖辦法第 1 條；見 §4）。

### 3.3 交割日曆（data-confirmed + official）
- `qc/calendar.parquet` 欄位：`trade_date`（每個平日一列，2022-01-03 → 2026-10-16）、`twse_open`、`tpex_open`（bool，可 null）。2022 起 TWSE 與 TPEx 開休市**完全一致**（0 列不同）。
- **交割日 ≠ 交易日**：TWSE 每年農曆春節前有 2 天「市場無交易，僅辦理結算交割作業」，在 `qc/calendar` 中 `twse_open=false`，但它們**是交割營業日**。資料範圍內：2022-01-27、2022-01-28、2023-01-18、2023-01-19、2024-02-06、2024-02-07、2025-01-23、2025-01-24、2026-02-12、2026-02-13。來源：TWSE 市場開休市日期 API（前一研究者下載 `dl2/h20xx.json`；URL 形式 `https://www.twse.com.tw/rwd/zh/holidaySchedule/holidaySchedule?date=YYYY&response=json`，unconfirmed 確切 URL），official。
  - 例：2024-02-05（春節前最後交易日）買進 → T+2 = 2024-02-07（僅交割日），不是 2024-02-15。
- **颱風休市**（資料中：2023-08-03、2024-07-24、2024-07-25、2024-10-02、2024-10-03、2024-10-31，以及 2025/2026 其他非國定假日的 false 日需逐一確認）：依 TWSE 公告「集中交易市場全日休市，當日應屆交割款券順延辦理」（新聞轉述 TWSE 公告；unconfirmed 原文）。即該日不算交割營業日，順延至下一營業日。除權息日遇休市亦順延一天（新聞）。
- 補行上班的週六：「不交易亦不交割」（TWSE 開休市表，official）。
- **建議**：`settlement_calendar = {twse_open == true} ∪ settlement_only_days`（settlement_only_days 需另存一個小表，marketdata 目前沒有）；`settle_date(T) = 第 2 個 > T 的 settlement day`。

### 3.4 賣出前的持股檢查（official）
營業細則第 79-1 條：證券經紀商接受普通交割賣出委託時，「應確認委託數量不超過委託人在其保管劃撥帳戶餘額」，例外包括（摘）：第 14 款「委託賣出**前一營業日買進**之有價證券」、第 15 款依當沖辦法委託賣出、第 13 款 ETF 相關。
→ 現股帳戶可賣數量 = 集保餘額（已交割）+ 前一營業日（及更早尚未交割）買進數量 + 當日買進且符合當沖資格者（§4）。**不可無券賣出**（除先賣後買當沖）。

### 3.5 違約交割（official 摘要 + unconfirmed 細節）
- 款券交割時限：T+2 上午 10 時前（TWSE 交易制度介紹）。未如期交付即「違約」，券商向交易所申報；券商得收違約金，**上限為原成交金額 7%**；當沖違約則以沖銷後差價為上限（2023 年《證券經紀商受託契約準則》第 19 條修正，中央社 2023-03-28 <https://www.cna.com.tw/news/afe/202303280334.aspx>；unconfirmed 生效日）。違約紀錄影響開戶（營業細則第 79 條「未有證券交易違約紀錄」為特定委託條件）。
- 處置股：券商須**預收款券**（營業細則第 82 條第 4 項；events `disposition`）。

### 3.6 Backtest 檢查建議
1. **下單時（硬性）**：賣出數量 ≤ 可賣數量（§3.4）；非當沖不得超賣。
2. **下單時（可選，參數 `cash_check = AtOrder | AtSettlement | None`，預設 AtOrder）**：`cash_now + Σ未交割淨應收付(含本筆, 依交割日排序後最壞時點) ≥ 0`。保守實作：對每個未來交割日 d，`cash_now + Σ_{settle ≤ d} net_flow ≥ 0`；任一 d 不成立就拒單（或標記 `SETTLEMENT_SHORTFALL`）。
3. **交割日（必做）**：`cash_balance += net_flow(d)`，若 < 0 → 記錄 `SettlementDefault{date, shortfall}`，backtest 應視為錯誤（現實會違約）。
4. 處置期間（`events.disposition`）的買進：要求下單時現金足額（預收款），賣出要求券已在集保（不得用 T-1 買進未交割的股票，unconfirmed：預收券是否接受前一日買進，需查處置作業要點）。


> §3.3 補充（data-confirmed）：`qc/calendar` 中 `twse_open=false` 而不在 TWSE 開休市表的平日＝颱風休市：2023-08-03、2024-07-24、2024-07-25、2024-10-02、2024-10-03、2024-10-31、2026-07-10。2025 年新增國定假日（09-29 教師節、10-24 光復節、12-25 行憲紀念日）均在開休市表中。

---

## 4. 現股當沖（Day trading）

### 4.1 法規（official）
- 有價證券當日沖銷交易作業辦法（TWSE 法規分享知識庫 FL071871 <https://twse-regulation.twse.com.tw/TW/law/DAT0201.aspx?FLCODE=FL071871>；**現行版修正日期 民國105-02-01**，2026-11-18 線上擷取與前一研究者副本逐條相同）：
  - 第 1 條：委託人就**同一受託買賣帳戶於同一營業日**，對主管機關指定之上市（櫃）有價證券，委託**現款買進與現券賣出同種類**有價證券成交後，就**相同數量部分相抵**之普通交割買賣，**按買賣沖銷後差價辦理款項交割**。
  - 經公告**變更交易方法（全額交割）、處置**之有價證券不得當沖。
  - 只限「普通交易收盤前之買賣間」及「普通交易收盤前之買賣與**盤後定價**交易間」反向沖銷。
  - **零股、鉅額買賣**等不適用（→ 盤中零股、盤後零股都不能當沖）。
  - 第 2 條：帳戶需開立滿 3 個月且最近一年成交 ≥ 10 筆（信用帳戶、專業機構等免）；須簽**概括授權同意書**與風險預告書；已簽者「於成交當日如不欲沖銷者，應於收盤前向證券經紀商聲明」（**預設自動沖銷**）。不得用綜合帳戶。
  - 第 3 條：當沖買賣金額列入單日買賣額度；未訂額度者另訂當沖額度（券商風控，broker-specific）。
  - 第 6 條：前月當沖累計虧損達額度 1/2 → 暫停當沖。
  - 第 7–9 條（先賣後買未回補＝**券差**）：券商出借股票交割，借券費率 ≤ 當日收盤價 7%；**T+1 由券商以「應付當日沖銷券差專戶」強制買回**，費用、價差由投資人負擔。
  - 第 12 條：遇停止過戶，自**停止過戶日前 5 個營業日起停止先賣後買**（股東會、除權息等）。
- TWSE 當日沖銷交易專頁 <https://www.twse.com.tw/zh/products/system/day-trading.html>：
  - **先買後賣**：2014-01-06 起；**先賣後買**：2014-06-30 起（official）。資料範圍內兩者皆可。
  - 標的：臺灣50、中型100、富櫃五十成分股；得發行權證、得融資融券、得借券之標的（經公告）；排除變更交易方法、處置。
- 證交稅：股票當沖 1.5‰（§2.2）；ETF、TDR 當沖仍 1‰。

### 4.2 marketdata 中的每日資格（data-confirmed）
- `instruments.f1_daytrade_flag`（只有 `source='F1'`，**2024-04-01 → 2026-10-12**）：TWSE 資訊傳輸作業手冊 格式一 3.1.14「可現股當沖註記」：
  - `A`：可先買後賣**或**先賣後買
  - `B`：**僅可先買後賣**
  - null（原始 SPACE）：不可現股當沖
- 實際分佈（F1 列）：STOCK A 953,460／B 42,330／null 171,464；ETF A 157,566／B 11,142／null 4,376；TDR A 1,659／B 43／null 758；ETN、REIT、權證 **全部 null**；OTHER A/B 只出現在 6 碼 ETF、91xxxx TDR。
- 與 `events.parquet` `event='daytrade_suspended'`（FinMind `TaiwanStockDayTradingSuspension`，「暫停先賣後買當沖」，2014-06-24 → 2026-11-17，37,437 列，`start_date`／`end_date` 皆有值，`reason` 例：股東常會 7,548、除息 6,206、分配收益 3,485、除權息 539、現金增資 407、臨時會 320、除權 178、減資 83、變更面額 19、ETF分割 6…）交叉比對：F1 為 `B` 的 53,711 列中 53,695 列落在 daytrade_suspended 區間內（只有 16 列 STOCK 2026-07-13 不在，疑似 events 缺漏）；落在區間內但旗標為 A 的 704 列（區間邊界差異）。→ **B ⇔ 暫停先賣後買** 高度一致。
- STOCK null（不可當沖）多與 `f1_abnormal_code`（1 注意、2 處置、3 注意及處置、4 再次處置、5 注意及再次處置）或非融資融券標的（新上市、全額交割等）相關；資料中**沒有獨立的「全額交割／變更交易方法」旗標**（unconfirmed；可由 null 旗標間接得知）。
- **2022-01 → 2024-03 沒有 F1 旗標**：只能近似 → `eligible_buy_first = instrument_type∈{STOCK, ETF, TDR, 6碼ETF/TDR in OTHER} ∧ 不在 events.disposition 期間`；`eligible_sell_first = eligible_buy_first ∧ 不在 events.daytrade_suspended 期間`。這個近似**高估**（無法排除非融資融券標的、全額交割股）→ 標 `eligibility_source = approx` 讓報表可區分。
- `daily_ext.daytrade_qty`、`daytrade_buy_amount`、`daytrade_sell_amount`（FinMind `TaiwanStockDayTrading`）可做事後檢查：某日某檔 `daytrade_qty` > 0 表示市場上確有當沖（間接證明可當沖）。

### 4.3 當沖的帳務（official 原則 + 建議實作）
- 判定：同一帳戶、同一交易日、同一 symbol，`board ∈ {REG（含 F6/F17 盤中整股）, FIX 盤後定價}`（不含 ODD）之買進股數 `B`、賣出股數 `S`；沖銷量 `q = min(B, S)`（以股計；整股單位 1000 股或 `shares_per_lot`）。帳戶開啟當沖（概括授權）時自動沖銷，除非策略明示「不沖銷」。
- 款項：T+2 只交割差價。由於 §3.2 的交割日淨額本來就合併同日買賣，ledger 只要把當沖兩腿與其他成交一起放進同一交割日淨額，結果即等於「差價交割」。差別只在**稅率**（沖銷量部分 1.5‰）與**風控**（當沖買進不需事先有現金，只要能在當日沖掉）。
- 未沖銷部分：
  - `B > S`（先買後賣未賣完）：剩 `B − q` 股成為一般買進，T+2 應付全額價款（現金不足 → 違約；backtest 需檢查）。
  - `S > B` 且非來自既有庫存（先賣後買未回補）：`S − q` 為**券差**，T+1 券商強制買回（價格不確定）、加借券費（≤ 收盤價 7%，實際費率 broker-specific）。**建議 backtest 直接拒絕**造成券差的賣單：先賣後買的賣單只在 `eligible_sell_first` 且策略保證收盤前（或盤後定價）回補時接受；收盤時若仍有未回補量 → 標記 `DAYTRADE_SHORT_UNCOVERED` 並以 T+1 開盤價（或參數）強制買回 + 借券費參數 `borrow_fee_rate`（預設 7% 上限）。
  - 若帳戶原本就有庫存，賣出可視為賣庫存（一般 3‰）或與當日買進沖銷（1.5‰）。在概括授權下券商會自動以沖銷處理（同數量）；最終持股相同，差在稅率與交割差價。建議：`auto_offset=true` 時一律先沖銷。
- 沖銷量在多筆不同價賣出成交間的分配（哪幾筆適用 1.5‰）：法規未規定，broker-specific → 參數 `daytrade_tax_allocation ∈ {FIFO_by_time, Pro_rata}`，預設 FIFO（unconfirmed）。
- 零股、權證、ETN、REIT：**不可當沖**（零股 official；權證/ETN/REIT data-confirmed：F1 旗標全 null）。ETF：可（旗標 A/B），稅率 1‰ 不減半。


---

## 5. 公司行動（持有部位受影響者）

### 5.1 marketdata 對應表（data-confirmed）
**`corp_actions.parquet`**（schema.md §4.7；唯一鍵 `(symbol, ex_date, action)`）欄位：`symbol`、`ex_date`（除權息日；減資／分割為恢復交易首日）、`action`、`before_price`、`ref_price`、`value`、`cash_dividend`（元／股）、`stock_dividend`（元／股，面額 10 元計：1 元＝每股配 0.1 股）、`subscription_price`、`factor`、`source`。

2022-01-01 起各 action 列數：`dividend` 11,247（含現金 10,720、股票股利 800）、`rights` 437、`reduction_loss` 107、`reduction_cash` 77、`par_change` 21、`reverse_split` 6、`split` 5。依類型：STOCK 7,496 dividend／ETF 3,317／OTHER 208（特別股、6 碼 ETF）／REIT 53／TDR 14／ETN 11。

範例列：
```
2330 2025-09-16 dividend  before 1255.0 ref 1249.99 value 5.000017 cash 5.000018 stock null
1101 2022-07-21 dividend  before 41.6   ref 36.91   cash 0.999283 stock 0.999283（每股配 0.0999283 股）
1236 2023-10-11 reduction_cash before 20.6 ref 23.25 factor 1.128641
1441 2023-08-29 reduction_loss before 7.64 ref 13.74
0050 2025-06-18 split before 188.65 ref 47.16 factor 0.249987（1 拆 4）
00632R 2024-12-11 reverse_split before 3.28 ref 22.96 factor 7.0（7 併 1）
2327 2025-08-25 par_change before 546 ref 136.5 factor 0.25（面額 10→2.5，股數 ×4）
2882 2022-12-05 rights before 44.0 ref 44.0 value 0.833205 subscription 35.0
```
**corp_actions 缺的欄位（ledger 必需，需另補）**：
- **現金股利發放日**：不在 `corp_actions`，但 raw `/data/raw/finmind/TaiwanStockDividend/{symbol}.json` 有 `CashDividendPaymentDate`（2022 起 11,763 筆，全部非空；除息→發放日距離 9–98 天，中位數 27 天、P95 35 天）（data-confirmed）。建議 marketdata 把 `pay_date` 加進 `corp_actions`。
- **股票股利新股交付（上市）日**：FinMind 無此欄位（unconfirmed 來源，需 MOPS／TDCC）→ 參數，例如預設＝現金發放日或 `ex_date + N` 營業日，交付前不可賣出。
- **減資比例、每股退還金額、退款日、換發新股日**：`corp_actions` 沒有。可由價格近似反推（減資前 10 元面額時）：
  - 彌補虧損：`ref = before / (1−r)` ⇒ `r ≈ 1 − before/ref`（1441：1 − 7.64/13.74 ≈ 44.4%）。
  - 現金退還：`ref = (before − 10·r)/(1−r)` ⇒ `r ≈ (ref − before)/(ref − 10)`（1236：(23.25−20.6)/(23.25−10)=0.2，即減資 20%、每舊股退 2 元）。
  - 因 `ref_price` 四捨五入到跳動單位，反推只是近似（unconfirmed）；**exact ledger 必須取 MOPS 公告的減資比例與退款**。
- **停止買賣期間**：`events.halt`（FinMind `TaiwanStockSuspended`）與 2022 年後 184 筆減資**沒有任何一筆對上**；實測 `daily` 在 `ex_date` 前有 4–7 個交易日沒有成交列（減資中位 7 天、分割／反分割 5 天、面額變更 7 天）（data-confirmed）。→ ledger 以「最後交易日 → ex_date」為凍結期：部位不能交易，但未交割款券照常交割。
- **合併、股份轉換、下市**：`corp_actions` 沒有；`events.daytrade_suspended.reason` 只見「合併」2 筆、「股份轉換」12 筆（只是暫停先賣後買）。→ 持有標的從 `instruments` 消失時 ledger 必須報錯／人工處理（對價可能是現金或存續公司股票）。

**`events.parquet`**（schema.md §4.15）：`symbol`、`event`、`announce_date`、`start_date`、`end_date`、`resume_date`、`reason`、`detail`。值：`disposition` 6,920（處置；需預收款券）、`halt` 9,408（暫停交易；`resume_date`，大多是權證）、`short_suspended` 39,097（暫停融券；現股帳戶不影響）、`daytrade_suspended` 37,437（暫停先賣後買當沖；`reason` 可當作公司行動的前兆：除息、除權、減資、變更面額、ETF分割、ETF反分割、合併、股份轉換、受益人大會、分配收益）。

### 5.2 現金股利（含 ETF 收益分配、REIT、TDR）
- **權利歸屬**（official）：營業細則第 67 條：於停止過戶日以後辦理交割者為除息交易。T+2 下，**`ex_date` 前一交易日收盤時的持股**（含尚未交割的買進、扣除尚未交割的賣出）享有權利；在 `ex_date` 當天或之後買進者無權利，`ex_date` 當天賣出者仍有權利。
- 帳務：`ex_date` 開盤前建立 `DividendReceivable{symbol, shares, rate, pay_date}`；`pay_date` 入現金。
- **每位股東金額**：公司公告標準文字「現金配發至元為止，**元以下捨去**，畸零款合計數列入公司其他收入」（例：智生活、環球晶公告；公司別規定，unconfirmed 為普遍，但 MOPS 公告慣例如此）。→ `cash = floor(shares × rate)`，`rate` 用 FinMind 原始全精度（如 2330 `6.00003573`）。
- **匯費**：發放時可能由股利中扣除匯費（常見 10 元，亦見 5 元，跨行時才扣；公司／銀行別）→ 參數 `dividend_remit_fee`（預設 0 或 10，broker/issuer-specific）。
- ETF 收益分配：與現金股利相同處理（`corp_actions` action=`dividend`；`events.reason='分配收益'`）。ETF 分配的進位方式由信託契約規定（unconfirmed；預設 floor）。
- **股利所得稅**：居住者個人不在發放時扣繳（併入綜所稅或 28% 分離），backtest 不建模（官方規定，非本研究重點）。

### 5.3 股票股利（盈餘／資本公積轉增資）
- 配股數＝`shares × stock_dividend / 10`（`stock_dividend` 以面額 10 元計；非 10 元面額股票需改用面額，`f1_non10_par` 標記）（schema.md 定義，data-confirmed）。
- 畸零股：公司公告慣例「不足一股之畸零股，股東得於停止過戶日起 5 日內辦理湊股；未湊足者**按面額折付現金至元為止**」（經理人雜誌報導、緯穎 2023 爭議報導；公司別，unconfirmed 為通則）→ `new_shares = floor(entitled)`、`cash_in_lieu = floor(frac × par)`。
- 新股交付日：不在資料中 → 參數；交付前不可賣出。
- 權利歸屬同 §5.2。

### 5.4 減資（現金退還／彌補虧損）
- 流程（data-confirmed 時間型態）：最後交易日 → 停止買賣約 5–7 個交易日 → `ex_date` 以新參考價恢復交易（新股）。
- 持股：`new_shares = floor(old × (1 − r))`；畸零股按面額退現（公司別）。
- 現金減資退款：`floor(old × refund_per_share)`，於公告的退款日入帳（不在資料中 → 參數或外部資料）。
- 彌補虧損：無現金流，只減股數。
- 成本：建議總成本不變（彌補虧損）；現金減資的退款視為 **return of capital** 扣減成本（§6 建議）。

### 5.5 分割、反分割、面額變更
- 股數 × 比例（0050 1→4、0052 1→7、00631L 1→22、00663L 1→7、00685L 1→24；00632R 7→1、00673R 4→1、00674R 5→1、00676R 6→1、00706L 4→1、00715L 2→1；面額變更 10→2.5 為 ×4、10→1 為 ×10、10→5 為 ×2）。比例可由 `before_price/ref_price` 取最近整數比得出（00676R factor 5.995 ⇒ 6；data-confirmed 近似）。
- 反分割的畸零單位：通常以現金補償（依發行人公告，unconfirmed）→ 外部資料或參數。
- 停止買賣約 5–7 個交易日（data-confirmed）。

### 5.6 現金增資（`rights`）
- 不認購的股東：**ledger 無需任何分錄**；除權只造成價格下調（`ref_price`），持股數不變。認股權利證書在台灣一般不上市交易（unconfirmed：少數例外）。backtest 若不模擬認購，直接忽略即可；還原價計算需記得 `rights` 也有 factor。
- 若要模擬認購：需繳款日、新股上市日（資料沒有）。

### 5.7 二代健保補充保費（股利）
來源：健保署「二代健保補充保險費簡介」<https://www.nhi.gov.tw/ch/dl-26429-47de962ef1464d8f91828d4421028290-1.pdf>（前一研究者下載並核對）；健保署 Q&A <https://www.nhi.gov.tw/ch/cp-2855-c7fc6-3150-1.html>；扣取及繳納補充保險費實務手冊 <https://www.nhi.gov.tw/ch/dl-64814-5f26c2b955c24ebbbbf1479877ecfb7e-1.pdf>。
| 項目 | 規則 | 生效 |
|---|---|---|
| 費率 | 1.91% → **2.11%** | 2.11% 自 **2021-01-01**（資料範圍全期 2.11%）（official） |
| 門檻 | 單次給付 ≥ **20,000 元** 才扣（達到門檻即全額計，非扣除 2 萬後） | 自 2016-01-01（official） |
| 上限 | 單次給付超過 1,000 萬元以 1,000 萬計 | official |
| 費基 | 股利總額：現金股利＋股票股利；同一基準日的現金與股票股利視為同一次給付（official，健保署實務手冊例：股票股利 26,000 元 × 2.11% = 549 元，全額計費） | 股票股利按**面額**計：通行規則，本次未取得健保署原文 → unconfirmed |
| 進位 | 元以下（角以下）四捨五入 | official（實務手冊） |
| 免扣 | 以雇主／自營業主身分投保者、第 5 類等（身分別） | official，建議參數 `nhi_exempt` |
- **建議建模**：個人投資人預設開啟：`premium = round_half_up(min(base, 10_000_000) × 0.0211)` 若 `base ≥ 20000`，自現金股利扣除（現金不足時次年由健保署收，backtest 可直接當日扣）。ETF 分配中屬股利／利息所得部分才計（組成比例需發行人公告，unconfirmed → 簡化為全額或參數）。


---

## 6. 成本與損益慣例

### 6.1 法規背景（official）
- 所得稅法第 4-1 條：「自中華民國七十九年一月一日起，證券交易所得停止課徵所得稅，證券交易損失亦不得自所得額中減除。」<https://law.moj.gov.tw/LawClass/LawSingle.aspx?pcode=G0340003&flno=4-1>
  → 台灣個人沒有資本利得稅，**沒有法定的 tax-lot（FIFO／特定批）規則**；成本法純屬報表慣例。
- 券商對帳單只要求逐筆列出價金、手續費、稅（營業細則第 80、93 條：買賣報告書與月對帳單格式由主管機關規定）。

### 6.2 券商實務（broker-specific，unconfirmed）
- 「庫存」頁多數以**加權平均成本（含買進手續費）**顯示持有成本；「未實現損益」以現價扣除**預估**賣出手續費與證交稅（多數用一般 3‰，不用當沖稅率）。
- 「已實現損益」：有券商以 FIFO 配對、有以平均成本計算；各家 App 不一致，搜尋未找到統一規範。
- 股利是否攤減成本：多數券商提供「含息／不含息」兩種顯示（broker-specific）。

### 6.3 建議的 exact convention（文件化、可重算）
1. **現金帳為唯一真相**：所有金額為整數元（§7）；P&L 最終以「期末權益 − 期初權益 − 外部出入金」驗證，任何成本法都不影響總 P&L。
2. **批次（lot）以 FIFO 保存**，每個 lot 記 `qty`、`cost_total`（整數元＝價金＋該筆分攤到的買進手續費）。當一筆手續費覆蓋多筆成交（PerOrder 聚合）時，按價金比例分攤、以最大餘數法讓整數加總等於實收。
3. **賣出已實現損益**＝`賣出淨價金（價金 − 賣出手續費 − 證交稅）− FIFO 釋出 lot 的 cost_total`（部分釋出按股數比例分攤，最大餘數法保持整數）。當沖配對的買賣先互相配對（同日 lot 優先）以符合當沖語意。
4. **同時輸出平均成本視圖**（不另存狀態，由 lots 推得），方便與券商「庫存」畫面比對。
5. 公司行動：
   - 現金股利／ETF 分配：記為 `income`，**不**調整成本（另提供「含息報酬」報表）。
   - 二代健保、匯費：記為 `expense`。
   - 股票股利：新增 `cost_total = 0` 的 lot（取得日＝交付日；FIFO 順序放在最後）；畸零股現金記 `income`。
   - 分割／反分割／面額變更：各 lot 股數乘比例，`cost_total` 不變；畸零單位現金補償視為該 lot 部分處分（已實現）。
   - 減資：彌補虧損 → 股數按比例減、成本不變；現金減資 → 股數按比例減、退款按比例**沖減成本**（return of capital，若沖減後為負則差額記已實現利得）。
   - 現金增資未認購：無分錄。
6. 所有規則以 `AccountingConvention` 版本號記錄在 backtest 輸出中。

---

## 7. 金額單位與進位

- **新臺幣現金一律整數元**（i64）：交割金額、手續費、證交稅、股利、補充保費都是整數元（official：外幣證交稅換算「計算至元」；健保費「以元為單位」；公司股利「配發至元」）。
- **價格**是十進位小數（跳動單位 0.01 起）；`price × shares`：
  - 整股（1000 股或 100 股／張）時一定是整數元。
  - **零股**可能有小數：TWSE「盤中零股精進措施」（2023-10-24 簡報 <https://dsp.twse.com.tw/public/static/downloads/brokerDepartment/A2--盤中零股精進措施_20231024092707.pdf>）載明：舊制「以成交筆數與投資人交割，**元以下無條件捨去**」，新制改為「**同一委託書相同成交價格**」合併後再捨去。例：600 股分 600 筆成交（25.65×230、25.75×75、25.85×295）舊制 15,000 元、新制 5,899+1,931+7,625=15,455 元（official 規劃）。各券商實施日不同：玉山證券 2024-01-19、Pocket 證券 2024-04-01（<https://www.pocket.tw/announcement/814/>）→ **broker-specific 參數** `odd_lot_amount_rule ∈ {PerFillFloor, PerOrderPriceFloor}` + 切換日期。是否買方也捨去（買方少付）：簡報只說賣方短少；推論買賣雙方同規則（unconfirmed）。
  - 手續費、證交稅以「捨去後價金」或「原始價金」為基礎：broker-specific（unconfirmed）。
- **每股股利多位小數**：FinMind 原始值最多 8 位小數（`CashEarningsDistribution`，例 2330 `6.00003573`）；`corp_actions.cash_dividend` 為同一資料（實測 10,720 筆中與 raw 最新公告值 0 差異）。每位股東：`floor(shares × rate)`（公司公告「元以下捨去」慣例）。為避免浮點誤差，ledger 應以十進位（例如 `rust_decimal` 或把 rate 轉為 10⁻⁸ 的整數）計算：`floor(shares × rate_e8 / 10^8)`。
- 股票股利率（元／股，面額 10）同樣用十進位：`entitled = shares × stock_dividend / 10`。
- 建議 ledger 型別：`Money = i64 (元)`、`Qty = i64 (股)`、`Price = i64 (0.0001 元，與 marketdata §4.3 萬分之一元一致) 或 Decimal`、`Rate = Decimal`。乘法後只在「規則指定的進位點」做一次進位。

---

## 8. 建議參數總表

| 參數 | 預設 | 來源類型 |
|---|---|---|
| `fee.base_rate` | 0.001425 | official（標準費率；超過需事前通知） |
| `fee.discount` (per board/channel/daytrade) | 1.0 | broker-specific |
| `fee.discount_mode` | Immediate（或 Rebate{Monthly}） | broker-specific（營業細則 §94 允許兩者） |
| `fee.aggregation` | PerOrder | broker-specific |
| `fee.rounding` / `rounding_stage` | Floor / AfterDiscount | broker-specific |
| `fee.min_regular` / `fee.min_odd` | 20 / 1 | broker-specific（法規只授權「每筆委託最低費用」） |
| `tax.rates` by tax_class | §2.3 表 | official（REIT 免稅、ETN 1‰ 為交易所說明） |
| `tax.daytrade_rate` / `until` | 0.0015 / 2027-12-31 | official |
| `tax.bond_etf_exempt_until` | 2026-12-31（草案擬延至 2036-12-31 並納入主動式債券 ETF，自 2027-01-01） | official / pending |
| `tax.aggregation` / `rounding` | PerOrder / Floor | unconfirmed |
| `daytrade.enabled` / `auto_offset` | true / true | 帳戶設定 |
| `daytrade.tax_allocation` | FIFO_by_time | unconfirmed |
| `daytrade.eligibility_source` | F1 flag（2024-04+），否則 approx | data |
| `daytrade.borrow_fee_rate`（券差） | 0.07 上限 | official 上限；實際 broker-specific |
| `settlement.lag` | 2 settlement days | official |
| `settlement.settlement_only_days` | §3.3 清單 | official（需另存） |
| `cash_check` | AtOrder | 設計選擇 |
| `odd_lot_amount_rule` + 切換日 | PerOrderPriceFloor（2024 Q1 起） | broker-specific |
| `dividend.rounding` | Floor（元） | 公司公告慣例 |
| `dividend.remit_fee` | 0（可設 10） | issuer/bank-specific |
| `dividend.pay_date` | raw `CashDividendPaymentDate` | data（需補進 corp_actions） |
| `stock_dividend.credit_lag` | = 現金發放日或 ex+N | unconfirmed |
| `fraction_cash_in_lieu` | floor(frac × par) | 公司公告慣例 |
| `nhi.enabled/rate/threshold/cap` | true / 0.0211 / 20,000 / 10,000,000 | official |
| `reduction.ratio/refund/pay_date` | 外部資料（MOPS）；可近似反推 | data gap |

## 9. 未確認事項清單
1. 證交稅與手續費的**計算粒度**（每筆成交／每委託／每日同股同別）與**進位**（floor／四捨五入）：法規未明定，需以實際券商對帳單校驗。
2. ETN 1‰、REIT 免稅的**法源條文**（目前依 TWSE 交易制度頁與投資指南說明；未取得財政部函令原文）。
3. 主動式債券 ETF（字尾 D）2027 起停徵：行政院草案（2026-10-01），尚待立法院三讀。
4. 107-04-27 版第 2-2 條的中間到期日（2021-12-31）未逐版核對（不影響 2022 起資料）。
5. 颱風休市日「當日應屆交割款券順延」：依新聞轉述 TWSE 公告，未取得公告原文。
6. 2022-01 → 2024-03 沒有 `f1_daytrade_flag`，當沖資格只能近似；資料中無「全額交割／變更交易方法」獨立旗標。
7. 股票股利新股交付日、減資比例／退款／退款日、反分割畸零補償、合併對價：marketdata 沒有，需外部（MOPS／TDCC）資料。
8. 零股價金捨去新制的各券商實施日，以及手續費／證交稅是否以捨去後價金為基礎。
9. 處置股預收券是否接受前一日買進未交割的股票。
10. 當沖違約金改以沖銷差價為上限的生效日（受託契約準則第 19 條，2023 修正）。

> 狀態：完成（2026-11-18）。
