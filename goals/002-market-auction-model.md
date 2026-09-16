# 002：統一連續交易與集合競價的市場狀態模型（核心先行）

Status: pending
Depends on: 001-remove-tui.md

範圍修訂：2026-09-16。先完成供應商中立的核心契約與執行流程；本次不取得背景資料、不擴充 Teralion mapping，也不進行整套 source 解耦。

## 1. 目標與範圍

以 **Continuous／CallAuction 兩種撮合方式**，建立 TWSE／TPEx 普通交易股票共用的市場語義。必須支援 opening、closing、delayed opening、delayed closing、處置分盤集合競價、處置競價延後，以及處置股的開收盤與其延後。正常盤中瞬間價格穩定措施亦需沿用相同模型，不另開一套流程。

成功標準不是「多出一組 enum」，而是供應商中立的 DomainEvent 與可選初始化資料，經過實際的 MarketState、TradingContext、strategy 與 execution simulation，取得一致解釋，而且 **用新模型替換既有重複分類與判斷**。六種情境以合成中立輸入驗收，不以 Teralion 是否提供完整資料作為本目標的完成前提。

遵循 `goals/README.md`：Astra low 決策與驗收，Sol medium 實作與撰寫測試，Luna max 執行測試、實驗及輪詢。不要求舊 API、設定或 cache 相容；必要跨模組修改可以一起完成，不留下新舊雙軌。

本目標明確授權把「可由已知資料推導的通用市場狀態」收回核心，不再要求各策略自行解碼 flags、關聯暫緩 trigger 與後續試算。同步修訂舊文件中相反的責任分配。但**不授權重建交易所委託簿、重算觸發門檻、產生不存在的成交，或保證真實撮合結果**。

只處理這條市場狀態路徑。保留 TAIFEX 與權證的既有有效能力，更新共用型別的必要呼叫端；不要把股票的價穩規則直接套到權證／期貨，也不要趁機擴增零股、盤後定價、即時交易或整套法規引擎。

**依使用者補充，目前 repo 的 normalizers 都屬於 Teralion source；按 TWSE／TPEx／TAIFEX 拆分，不表示它們是供應商中立的市場層。** 本目標不得因名稱而把這些 normalizer 當成核心的上游依賴。只允許共用型別變更所需的最小編譯／語義配合，不調查 Teralion 新欄位、不呼叫其 API、不補處置來源或建立 profile 載入流程。

## 2. 先固定正確的抽象邊界

### 2.1 撮合方式、競價目的、延後、處置身分，是不同問題

- **撮合方式**：逐筆連續撮合，或集合競價。重用既有 `MatchingMethod::{Continuous, CallAuction}`，不要再建立同義 enum。
- **競價目的**：Opening、Closing、Periodic、VolatilityInterruption。`Periodic` 表示分盤／週期性競價，不直接等同「處置」。
- **是否延後**：同一次競價的觀測屬性，不是另一種撮合方式，也不是 `DelayedOpening` 等一串新競價類別。
- **處置身分**：商品在該交易日的背景資訊；開盤、收盤與盤中競價都可能發生於處置股，不能被互斥的 `AuctionReason` 吃掉。

因此不要使用 `Opening / DelayedOpening / Closing / DelayedClosing / Disposal / DelayedDisposal / ...` 這種不斷擴張的單一分類。未知、收市及尚未觀測到狀態，也不能硬塞成 Continuous；它們不是第三種撮合機制。

### 2.2 這不是把所有 DomainEvent 改成兩個 variant

`QuoteSnapshot`、`BookSnapshot`、`TradeBatch`、`MarketStatus`、`IndicativeAuction` 回答的是「本筆資料提供什麼」。Continuous／CallAuction 回答的是「市場如何撮合」。保留既有有用的 payload 邊界，以共用語義 metadata 連接兩者，不為了二分法重寫整套事件格式。

尤其：**集合競價不等於試算資料**。集合競價期間有 indicative observation，競價完成時也會有正式成交／行情。試算與正式資料不能混用。

### 2.3 「支援」的可驗收定義：核心能力與來源覆蓋分開

本次必須做到：能建立中立輸入、能更新跨事件狀態、能供策略讀取、能正確限制模擬成交。用一般 Rust 值直接建立合成 DomainEvent 與可選背景，走 production 使用的同一條 reducer／runner／simulation 路徑；不能另寫只有測試使用的狀態機，或六種情境全部回傳 Unknown。

**本次完成的是核心六種情境的能力，不是 Teralion 六種情境的完整辨識／取得能力。** 既有來源已能表達的資訊必須保留；尚未提供的資訊維持未知。不得為了完成本目標新增資料抓取、背景檔案格式或 provider conformance 工作。

缺少處置身分或無法判定競價是否結束時，必須保留未知並指出缺少的證據。模型不能憑空還原全部歷史。報告分開列出「中立核心案例通過」「既有來源離線回歸結果」「尚未接入的來源能力」，不可把前兩者說成全市場真實資料已認證。

## 3. 已核對的制度與資料事實

以下是設計依據，不是要求實作所有交易所規則：

1. 一般股票開收盤採集合競價；盤中原則採逐筆，瞬間價格穩定措施可能暫緩後以集合競價撮合。[R1]
2. 處置／分盤證券也可能有暫緩開收盤及盤中價格穩定措施。TWSE 自 **2022-09-26** 起相關資訊揭露與配套措施已有明文說明，因此不能用較舊的「處置股不適用」說法排除 delayed disposal。[R2]
3. TWSE Q&A 明確說明：暫緩開收盤期間，不因再次達到波動標準而連續再延一次。因此「模型可以接收多次有證據的延後更新」不等於「目前開收盤制度會反覆延後」。[R3]
4. Repo 的 TWSE／TPEx 介面記錄區分 trial bit、僅在 trial 狀態有效的延後開收盤 bits、撮合方式、開收盤 markers 與 instant-trend。暫緩 trigger 可能是非 trial、零量 recent-price sentinel 與空簿，之後的 trial 未必重複攜帶 trigger 的方向。[C4][C5]

本次不重算 3.5% 等觸發門檻，也不寫死分盤間隔或「現在加兩分鐘就恢復」。交易日期、商品與制度版本可能不同；以已取得的來源觀測及當時已知的商品背景資料為準。現行說明不能無條件倒套歷史資料。

## 4. 推薦架構：從中立輸入開始，不從 Teralion payload 開始

```text
本次 production 契約與驗收入口

中立 DomainEvent + MarketSignal      可選的中立初始化值（記憶體傳入）
              │                                      │
              └──────────────┬───────────────────────┘
                             ↓
             market-state：每商品／交易日的單一 reducer
                             ↓
             MarketPhase + firm / indicative state
                             ↓
             TradingContext → strategy / execution-sim / runner

既有 Teralion normalizer → 中立 DomainEvent
  僅維持既有已知語義與必要型別配合；不新增資料能力。
未來其他 source → 同一個中立 DomainEvent
  本次不實作，不新增通用 provider framework。
```

責任分配：

| 位置 | 本次應承擔的責任 | 不應承擔的責任 |
| --- | --- | --- |
| `market-types` | 共用訊號、競價目的及必要 codec | Teralion payload 形狀、raw bit 解碼、背景資料取得 |
| `market-state` | 單一 reducer、接受可選初始化值、跨事件關聯與重設 | 查 source、讀設定檔／profile 檔案、價格觸發門檻與計時推測 |
| `strategy-api` | 已解析的唯讀狀態及必要交易資格 | 再解 raw flags 或建立另一套目的／延後分類 |
| `execution-sim`／runner | 接受中立輸入，使用共用狀態及本筆成交證據 | 替策略猜處置背景、針對六種名字各寫一條流程 |
| 現有 Teralion normalizers | 必要 imports／constructor 配合，傳遞原本已確定的資訊 | 新資料探索、額外 mapping、歷史認證、為本 goal 補齊背景來源 |

**新核心不 import、不呼叫既有 Teralion normalizer。依賴方向是來源實作使用核心契約，不是核心透過來源實作辨識市場。** 用既有測試 harness／iterator 建立中立輸入即可；不新增「通用 TWSE normalizer」、fake provider service 或第二套 adapter hierarchy。

這不是承諾本次讓整個 repo 完成供應商解耦。現有資料下載／source repository 架構留到後續目標；這次先讓新市場語義不再依賴那些細節。

## 5. 最小資料模型

以下是責任與形狀的建議，非可直接貼上編譯的完整 patch。可配合既有命名調整，但不得藉此擴張成另一套 framework。

### 5.1 事件側：小型語義訊號，不新增六種 DomainEvent

放在 `market-types` 的既有模組，或最多一個清楚的市場語義模組：

```rust
// 既有 MatchingMethod、Observation<T>、StabilityDirection 繼續重用。
pub enum AuctionPurpose {
    Opening,
    Closing,
    Periodic,
    VolatilityInterruption,
}

pub struct AuctionObservation {
    pub purpose: Observation<AuctionPurpose>,
    pub delayed: Observation<bool>,
}

pub enum MarketSignal {
    Continuous,
    AuctionCollecting(AuctionObservation),
    AuctionUncross(AuctionObservation), // 已觀測到本次競價結果／完成邊界
    Closed,
}

// 置於既有 event / annotations 邊界：
// market_signal: Observation<MarketSignal>
```

`AuctionUncross` 是**既有行情事件的 metadata**，不是另一筆重播事件。不得把同一 source observation 拆成 status、trade、book 三次 callback，或再插入一組人造 `AuctionStarted/Ended` 事件。

`Observation` 的三種語義要真的保留：`NoObservation`＝本筆沒提供；`Unknown`＝本筆指出無法確定／原先知識失效；`Set(value)`＝有明確資訊。不要把它們全部壓成 false 或把 Unknown 當成「沿用之前」。

`delayed=true` 只表示本次競價已觀測到延後；不代表延了幾次、延多久或精確恢復時間。方向沿用既有 neutral direction／trend 欄位，不複製成 `DelayedUpOpening` 等 variant。沒有需求與來源證據時，不新增 deadline、計數器、巢狀 interruption stack。

### 5.2 狀態側：沿用既有 StateField 的未知與來源語義

放在 `market-state`，不反向讓 `market-types` 依賴它：

```rust
pub struct AuctionState {
    pub purpose: StateField<AuctionPurpose>,
    pub delayed: StateField<bool>,
}

pub enum MarketPhase {
    Continuous,
    Auction(AuctionState),
    Closed,
}

// 加入既有 per-instrument state；不是新建另一個 state manager：
// market_phase: StateField<MarketPhase>
// intraday_matching: StateField<MatchingMethod>
// disposal: StateField<bool>
```

`market_phase` 是目前觀測到的階段；`intraday_matching` 是當日已知的盤中基本撮合制度。兩者不同：一般股票開盤時 phase 可以是 Auction，但其 intraday matching 是 Continuous；處置分盤股的 intraday matching 則是 CallAuction。

`disposal` 獨立保留，避免 Opening／Closing 蓋掉處置身分，也避免把所有分盤交易都叫處置。`false` 必須有資料支持；未提供就未知。

不另存與 phase 同義的 `is_auction`、`is_continuous`、`is_delayed_open` 等 flags。`matching_method()` 可由 phase 直接推得；未確定／Closed 回傳不可撮合或 unknown 的既有表示，不默認 Continuous。

### 5.3 可選初始化值：只收資料，不負責找資料

**刪除原版要求新增 dated profile 檔案、reference/config loader 與補齊來源的工作。** 本次只讓核心能接收呼叫端已經知道的背景值，缺值也能啟動 replay；核心不查它來自哪家供應商。

優先在既有每商品／交易日的初始化輸入增加兩個可選欄位；若沒有合適的輸入 struct，最多新增一個無行為的普通 struct：

```rust
// 初始化資料，不是增量事件；None = 初始化時沒有這項已知資訊。
// 型別／欄位名稱可配合現有程式，不是要求新增 service 或 trait。
#[derive(Debug, Clone, Copy, Default)]
pub struct MarketBackground {
    pub intraday_matching: Option<MatchingMethod>,
    pub disposal: Option<bool>,
}
```

例如合成處置案例直接傳入 `Some(CallAuction)` 與 `Some(true)`；未提供時使用 default，兩欄都是 None。`Some(false)` 才表示呼叫端明確宣告非處置；不從缺值推成 false。這是初始化專用的 Option，不能替換事件增量使用的 `Observation::{Set, NoObservation, Unknown}`。

必須落實的接口邊界：

- 從既有 production 的 per-instrument／session 初始化入口接收，不用 `#[cfg(test)]` setter，不讓策略在 callback 中任意改市場狀態。沿用既有 instrument／trading date／session 識別，不新增全域 lookup manager。
- 用既有 runner／整合測試入口把這些值傳至真正的 state 與 simulation；不能只做到 enum 能建構、卻無法在核心回測使用。不另造 orchestration API 或多一套 runner。
- 初始化的背景限於該商品、該交易日，跨日重新提供；僅能使用在該次 replay 可見窗口開始前已知的資訊。不允許把事後已知值預載到更早事件。盤中背景更新、來源優先序與多來源合併不在本次實作。
- 沿用現有初始狀態／run identity 機制記錄實際傳入值及作用域；需要時把這個小型可序列化值納入既有 checksum 輸入，不另建 profile manifest／provenance framework。不可把初始化值偽裝成有 event fingerprint 的行情；若現有欄位只能記行情 provenance，使用明確的 initialization origin。
- 來源 cache 仍只保存中立事件與既有 lineage；可選背景於初始化時套用，不把 strategy config 或整份當日背景灌進 normalizer cache。重播相同結果所需的初始化值隨既有 run 設定快照／輸入摘要保留。

本次**不新增 YAML／CSV／JSON 背景檔案格式、config 欄位、CLI 參數、檔案 loader、provider trait、資料查詢服務或外部 API 呼叫**。也不要求研究 Teralion 是否存在 disposal 欄位。測試直接用中立 Rust 值；既有 CLI 沒有這項背景時維持未知，後續來源接入另立目標。

缺少背景只降低能判定的資訊，不該讓資料 replay 本身無法啟動。例如可知道「正在 Auction」，但 purpose／disposal 未知；若 event 已明確提供 purpose，仍直接使用。只是 purpose 不明，不可一律當 Continuous，也不可一律把所有獨立已知資訊判成無效。只有交易政策確實依賴未知資訊時，才保守限制相應動作並保留診斷。

## 6. 六種必要情境的統一表示

下表的 false 表示有明確未延後證據；資料不足應是 Unknown，而不是猜 false。

| 使用者情境 | 撮合方式 | AuctionPurpose | delayed | disposal |
| --- | --- | --- | --- | --- |
| open | CallAuction | Opening | false | 可真／假／未知 |
| close | CallAuction | Closing | false | 可真／假／未知 |
| delay open | CallAuction | Opening | true | 原值保留 |
| delay close | CallAuction | Closing | true | 原值保留 |
| disposal 盤中分盤競價 | CallAuction | Periodic | false | true |
| delay disposal 盤中延緩 | CallAuction | Periodic | true | true |

處置股的 delayed open 是 `Opening + delayed=true + disposal=true`，不是把 Opening 改成 Disposal；delayed close 同理。

一般股票盤中明確的價穩 trigger，走 `VolatilityInterruption + delayed=true`。已知分盤股的盤中競價遇價穩，則仍是 `Periodic + delayed=true`；不要覆蓋它原本的制度背景。來源不足以區分時，保留已確認的 auction／delay 與未知 purpose，不能假裝辨識完成。

中立輸入不要求 normalizer 取得背景：合成盤中價穩 trigger 若沒有直接目的證據，可輸出 `AuctionCollecting`、`purpose=NoObservation`、`delayed=Set(true)`，並保留已中性化的價穩／方向訊號。Reducer 再依已知前態與盤中基本制度解讀目的；不能讓兩邊各推導一次，也不能把 `delayed=true` 本身當成所有價穩取消政策的充分證據。

## 7. 正規化與 reducer 契約

### 7.1 中立事件的輸入契約（不要求本次擴充來源 mapping）

以下界定核心可以相信什麼、測試應如何構造中立輸入；既有來源只保持原本已驗證的資訊傳遞。涉及 wire 的敘述是既有回歸界線，不是要求重新調查 payload。

- 所有正式標示 trial 的價量／五檔都進 `IndicativeAuction`，不進 firm snapshot／trade。trial 的 cumulative volume 也不得因方便而覆寫 firm volume。[C4][C5]
- status-only 暫緩 trigger 沿用 `MarketStatus`：可以攜帶競價／延後訊號，但零量 recent-price sentinel 不是成交，空陣列不是「正式清空五檔」。
- `trial=false` 時，延後開收盤 bits 無意義，不得據此設 delayed=true，也不得據此把已知 delayed 清為 false。
- 開／收盤明確標記與有效的 delay flags 優先於時間猜測。時間只能在已驗證的 session contract 下輔助解讀，不能單靠「過了 09:00／13:30」宣告開收盤完成。
- `AuctionUncross` 需要可靠的正式 auction result／完成標記，或已驗證具有此語義的正式成交。只有 CallAuction bit、普通 book update 或時間流逝，都不是充分證據。明確的無成交結束標記可以結束競價，但不能生出 fill。
- 保留既有 intermediate／final group 的原子性、成交量與排序驗證；同 timestamp 的多筆資料不得造成同一次競價重複結束。缺乏來源順序時，不以猜測的生命週期重新排列資料。
- 原始 bytes 的判讀屬於來源邊界。新 reducer／TradingContext／simulation 只讀 neutral 語義，不呼叫 Teralion decoder。共用型別調整所需的既有 flags 轉碼可在來源邊界沿用已驗證邏輯；不得擴充原始欄位判讀。現有 raw 診斷載體可暫時保留供來源追溯，但不得成為第二條核心狀態判斷路徑，也不為搬走所有 raw 型別重排整個 workspace。

### 7.2 Reducer 是跨事件關聯的唯一地方

只使用目前及已經套用的事件、當時已知的背景資料；不得掃描未來找真正開盤、收盤或下一筆成交時間，再回填 earlier state。

| 觀測 | 必要更新 |
| --- | --- |
| 明確 Continuous | phase 轉 Continuous，結束先前競價上下文；不抹除當日 disposal 背景 |
| AuctionCollecting | phase 轉／維持 Auction；按 observation 語義更新 purpose 與 delayed |
| 已知競價中的普通 trial，沒重複 trigger | 在可確認同一輪的前提下，保留先前已知目的與延後資訊；不因 flags 歸零失憶 |
| AuctionUncross | 先解析本次完成的競價資訊，再決定後續 phase；保留本筆正式行情的原子套用 |
| Closed | 關閉新委託資格；不抹除最後正式成交／五檔，不把 Closed 偽裝成 Continuous |
| NoObservation | 不更新對應資訊，不刷新該欄位的證據時間 |
| Unknown／明確資料斷裂 | 使受影響的知識失效；不得沿用過期的交易許可 |

競價完成後：

- **Closing → Closed**，包括 delayed close。
- **Opening／盤中競價 → 已知的盤中基本制度**。一般股回 Continuous；分盤股回 `Auction(Periodic)`，開始下一輪且清除上一輪的 delay 內容。下一輪是否未延後，依現行來源契約／證據決定；不足則 Unknown。
- 基本制度未知時，不猜回 Continuous。可由後續明確觀測確認；必要時 phase 保持 unknown。
- 分盤競價完成後不必先造一個瞬間 Continuous 再回 Auction。

新一輪、明確 Continuous、trading date／session 邊界，必須正確重設舊 auction context。保留歷史 firm price 不等於沿用舊競價狀態。中途進入一輪競價時可以知道「正在競價」但不知道開始時間、延後次數；不要補造。

只在確認同一輪時合併部分觀測。已確認從 Opening 進入新的 Closing，或前一輪已完成時，不得把舊 `delayed=true` 帶入新競價；缺少新欄位就 Unknown。有矛盾目的但沒有合理轉換證據時，走明確錯誤／未知處理，而不是默默合併。呼叫端初始化背景若與明確撮合訊號矛盾，也要保留診斷，不可無聲覆蓋。

### 7.3 延後更新與重複訊號

同一輪多筆 `delayed=true` 是狀態重申，**不是每筆都新增一次延後**。同值更新不增加輪次；不要新增 delay counter 來猜測市場事件。

模型不在 enum 中硬編最多延後一次。如果另一來源或未來已核對的制度真的提供同一輪多次延後通知，按現有事件順序接收即可；只在有獨立通知／修訂證據時辨認為新更新。相關合成測試須標成「模型表達能力」，不能當成目前 TWSE 開收盤制度的真實案例。[R3]

若某一盤結束後下一盤又延後，那是下一輪，不是上一輪一直累加。沒有結束／新輪證據時，不從間隔時間猜輪次。

## 8. 分清「本次競價結果」與「事件後市場狀態」

這是不可省略的邊界：

```text
開盤結果：本筆撮合 = CallAuction；事件後可能 = Continuous
收盤結果：本筆撮合 = CallAuction；事件後 = Closed
處置盤結果：本筆撮合 = CallAuction；事件後仍 = Auction(Periodic)
```

不能因事件後 Continuous，把開盤結果當連續撮合；也不能因事件後 Closed，把在收盤前已有效掛入的合格委託全部排除於收盤結果之外。

優先從現有 event metadata 取得本筆正式撮合性質。若需要帶出由前態解析出的 purpose／delay，**隨既有 apply summary／EventOccurrence 帶出一個本筆專用的可選 auction context 即可**；不要另建 event stream、歷史表或第二個 lifecycle manager。這個回傳值只是本次 transition 的結果，不是另一個狀態擁有者。

維持原有 callback 與 occurrence 的因果限制：看見本次 auction result 後才建立的委託，不能回頭成交在同一次 auction；同一 `match_time` 也不等於有資格回填。

## 9. 策略與模擬成交的整合要求

### 9.1 策略

`TradingContext`／`MarketStateView` 直接提供上述已解析狀態與背景；不要讓策略重跑 normalizer 或讀 `TwseQuoteAnnotations`／`TpexQuoteAnnotations`。

策略只為自己需要的動作讀取必要資訊，例如「正在 Auction」「purpose=Periodic」「delayed=true」「disposal=true」。不要要求每個策略寫六種特例，也不要新增六個 callback。

最新 indicative observation 可以保留既有欄位，但「曾有試算資料」不是「現在仍在競價」。目前 phase、試算資料時間與本次 result 要能區分。

### 9.2 Fill 與 order entry 分開判斷

競價收單中可以有符合已支援規則的委託，但**不因此可以立即以舊五檔或試算價成交**。市場狀態不明時，不能默認允許撮合。

`AuctionCollecting` 不等於所有時刻、所有 order type 都 `Allowed`。例如 TWSE 延後收盤的說明是 13:31 起再接受申報，不能為修正 nominal CoolDown 而把 13:30–13:31 也無條件開放；order-entry 仍由已驗證且適用該交易日的簡單市場政策判斷，不從「正在競價」直接推出許可。[R1][R3]

AuctionUncross 的成交只使用所選模型允許的正式成交證據、價量上限、委託資格與既有配置。已存在的 auction strict-cross／保守同價不成交等模型規則不應被此次分類重構悄悄放寬。[C6]

普通與 scheduled 路徑必須共用新語義。觸發延後／暫緩之後，保留供研究讀取的舊 firm book，但不可繼續把它當作仍可立即成交的深度；狀態變動不刷新 firm book timestamp，也不補回已消耗量。復原後仍需模型要求的正式、可用資料，不能拿 pause 前的 book 自動恢復撮合。

只維持本專案已支援的 order types 與已驗證限制。既有 market ROD 在明確價穩 trigger 下的取消行為、limit ROD 的處理與未 activation request 的區別須保留；不要因為統一成 Auction 就對所有競價無差別套用同一取消政策。IOC／FOK、完整預收款券規則與券商風控不在本 goal 新增。

### 9.3 時間邊界與多商品

- **SessionPhase 不等於 MarketPhase。** 目前 `SessionSegment::phase` 會在 nominal close 之後回傳 CoolDown；這不能先於市場訊號把 13:30 後仍有效的 delayed closing 直接封死。[C3]
- replay/data window、使用者明確設定的策略活動範圍、個股實際撮合狀態要分清楚。在資料窗口內，支援 delayed close 期間的正確資格與正式收盤結果；不要把所有商品一律延長收盤，也不要繞過使用者明確的策略時窗限制。
- 目前文件的普通交易 replay window `[08:55, 13:35)` 是可檢查的入口，不是所有歷史情境的永遠正確常數。資料窗口不足就明確報告，不截掉結果後假稱完成。[C4][C5]
- 一檔股票延後不得阻塞其他商品或 TAIFEX。仍沿用既有 deterministic streaming merge。
- scheduled 路徑保留現有 `match_time`／visible time／control time 區別；新狀態不得在 market-data latency 到期前洩漏給策略。不要為這個目標另建第二個行情時鐘或重寫整套 latency model。[C6]

## 10. 應替換的既有設計與必要來源配合上限

原版檢查的參考 revision：`b403ce6e5b04d67c6687ca33567ff6e02ebdb173`。本次修訂根據原 goal 與使用者澄清，不宣稱重新檢查了最新 codebase。執行時核對 working tree，先完成 goal-01，不回復使用者修改。

| 定位入口 | 本次收斂方向 |
| --- | --- |
| `crates/market-types/src/annotations.rs` | 重用 `MatchingMethod`；新核心語義不依賴 raw wrappers；不借此進行整個 source package 搬遷 [C1] |
| `crates/market-types/src/event.rs` | 替換重複的 `IndicativeAuctionKind` 分類；保留 firm／indicative payload，更新實際受影響 codec [C2] |
| `crates/strategy-api/src/context.rs` | 用共用 reducer 結果取代 `IndicativeReason` 及 TWSE／TPEx 重複狀態推導；必要 order policy 仍保留 [C3] |
| `market-state`、`execution-sim`、`osmium-runner` | 中立事件入口、可選初始化值、單一 reducer、必要 apply metadata；普通／scheduled 使用同一市場語義 [C6] |
| 現有 Teralion normalizers 與相關 fixture tests | 共用型別改變時做最小機械性配合，傳遞原已確定的資訊；不新增 wire 解讀或來源覆蓋 [C4][C5] |

**不要求來源程式碼零 diff，但禁止來源功能擴張。** 允許修 imports、constructor 參數、型別映射及既有確定 facts 的轉碼；例如原本已辨識為 opening trial 的資料，改建構新的 Opening auction signal。沒有原本可支持的資訊就維持未知，不查網路、不向 source 要新欄位。若必要介面配合確實無法和新來源研究分開，記錄具體依賴，不以猜測 mapping 掩蓋；仍完成可獨立驗收的核心部分，並如實標示未過的必要回歸。

新增相似型別時，指出哪個舊分類被刪除／替代；不得同時留下 `IndicativeAuctionKind`、`IndicativeReason` 與第三套同義分類，也不能為避免改 constructor 把新模型放成永遠不會被使用的旁路。保留的 raw 診斷欄位不代表允許保留舊 reducer／策略判讀路徑。

允許更新 event schema、canonical encoding、normalizer mapping、state／rule identity 等實際受影響的版本。更新 mapping identity 若只是同一來源資訊改編碼，要如實說明，不暗示新增來源能力。舊 cache 明確拒絕並離線重建，不維護相容層。未改行情與帳務語義仍需回歸對照；checksum 變動不可只改 golden value 當驗證。

同步更新產品需求、replay／execution model、實際受影響的範例、測試與介面文件，區分核心支援和現有來源覆蓋。不要新增 source／NAS／metadata 服務或爬蟲，也不要刪除使用者原始行情。

## 11. 防止過度工程的硬限制

- 不新增 crate、第三方依賴、通用 FSM／rule engine、事件匯流排、plugin registry、DSL 或排程 framework。現有 enum、struct、純函式／match 與 reducer 足夠。
- 不新增 profile loader、背景檔案格式、外部查詢、Teralion mapping 功能、通用市場 normalizer 或 mock provider framework。背景輸入最多是一個普通值型別，已存在等價初始化結構時不新增。
- 不為每種競價建立 trait、handler class、callback 或文件目錄。沒有第二個實際用例的 generic 不新增。
- 不持有全市場歷史來辨認狀態；每商品只保留目前必要上下文。沿用既有 event identity、state version、來源追溯與 trace，不另造 auction UUID／episode database。
- 不建立「推測撮合時間」排程器。沒有真實結果就不自行恢復，也不把輪詢或計時當行情。
- 必須刪掉被替換的特殊判斷與 dead code。回報 production code／tests 的分開增減與被刪除的概念；不設生硬 LOC 目標，也不靠刪測試降低行數。
- 每一項新狀態都要能對應本檔的驗收案例。完成這些能力就停止，不預先設計所有市場、所有監管事件。

## 12. 驗收案例

優先在既有測試中做表格化／參數化案例。六種情境直接建立中立 DomainEvent 與可選初始化值，經過 production 共用 reducer，再以少量案例走真實 strategy／simulation／runner。只用既有 Teralion fixtures 做回歸，不新增來源 conformance 或每列一套 fixture framework。

| 案例 | 必須證明的結果 |
| --- | --- |
| 正常 open | 中立初始化提供已知 Continuous 基本制度，試算→正式 auction result→Continuous；本筆結果仍是 CallAuction |
| 正常 close | Closing 試算→正式結果→Closed；既有合格單可依模型使用結果，新單不可回填 |
| delay open | Opening 身分保留，delayed=true；09:00 不自動恢復，等正式證據 |
| delay close | 13:30 後不因 nominal CoolDown 截掉延後競價；仍遵守適用的停收／恢復申報時窗，正式結果後才 Closed |
| disposal 連續兩盤 | 記憶體初始化傳入 CallAuction＋disposal=true，經真實核心入口完成兩盤；第一盤結束後仍是 Periodic，下一盤不繼承 delay |
| delay disposal | Periodic→delayed=true→正式結果→下一輪 Periodic；不跳回 Continuous |
| 處置股延後開／收盤 | purpose 是 Opening／Closing，disposal=true 同時保留 |
| 一般股票盤中價穩 | status-only trigger→無重複方向的 trial→正式結果；狀態不失憶、不依兩分鐘自動恢復 |
| 重複 delay 註記 | 多筆試算持續 delayed=true 不被當成多次延後或多輪競價 |
| 多次明確延後更新 | 合成、provider-neutral 訊號可順序處理；標明不是現行開收盤連續延後的制度案例 |
| 缺少背景／中途起播 | None 可啟動 replay，不猜 disposal=false、Periodic 或完整起始時間；中立輸入有足夠證據時不得全是 Unknown，事後背景不可提前初始化 |
| 非 trial 的 delay bits／保留位 | 不誤讀 delay；矛盾或未知訊號按既有 strict／degraded 契約處理，不默認交易許可 |
| 空簿 trigger／試算更新 | 不清掉 firm book，不刷新其 timestamp，不用 indicative price／volume 作正式成交或 mark |
| 正式無成交結束／不明 book update | 已驗證的結束 marker 可結束但不產生 fill；普通 book update 不被誤認為新一次 uncross |
| 同 timestamp 與來源分組 | 一次實際競價不被重複結束／重複消耗量；維持既有可重現順序及 source group 驗證 |
| 跨競價／跨日與多商品 | Opening 的 delay 不洩漏到新的 Closing、次日或其他商品；NoObservation 與 Unknown 分別驗證，背景按交易日切換，TAIFEX 原能力仍通過 |
| scheduled／資料延遲 | 狀態限制與 depth 分離更新，不能使用 pause 前深度或提前知道訊號；看到結果後不得回填成交 |
| 序列化與重播 | 新訊號與實際初始化值／作用域進既有 identity；相同初始化下 round-trip、重複執行與 fresh/cache replay 語義一致 |
| 供應商獨立驗收 | 中立核心案例不 import／呼叫 Teralion normalizer，不需 credentials、外部行情或新背景檔案；不是另寫測試替身 |
| 來源配合未擴張 | 既有 fixture 回歸保留既有已知語義；source diff 只有必要介面／原 facts 轉碼，沒有新增抓取、欄位探索或完整六情境來源認證 |

完成條件：六種中立情境經過實際核心 API／reducer／策略與模擬流程，含可選初始化與缺值路徑；普通／scheduled 不各自重做分類；必要既有來源回歸通過；被替換核心分類刪除；文件明確揭露目前來源缺口。**不再要求兩個 Teralion 股票 normalizer 為本次補齊六種情境，或 CLI 新增背景輸入。** 不能反向以這項範圍豁免刪掉原來已支援的來源語義。

使用既有 harness 以 provider-neutral synthetic events 走核心 replay／backtest 整合驗證，必要時檢查既有 artifact 輸出；另跑原有離線 CLI smoke／inspect 作為非回歸檢查。不為測試新增 CLI source 或資料服務，不呼叫 Teralion API、不下載行情、不跑全市場 benchmark。只使用 repo 內既有 fixtures，不把本目標擴成真實來源認證。

先 focused，再 workspace：

```sh
cargo fmt --all --check
# 使用目前 Cargo.toml 的實際 package 名稱，測受影響的 types / normalizers / state / strategy / sim。
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Astra 先確認核心入口、必要來源配合上限及關鍵新契約；Sol 完成一個必要且自洽的跨模組改動；Luna 對穩定 working tree 執行離線驗證並分開回報核心六情境、既有來源回歸的命令、exit code 與差異。失敗、無法執行或缺少證據的必要驗收不得標成通過。不因 breaking change 再次請求相容性授權。不自行 commit／push。

## 13. 來源與查核界線

原版查核標示日期：2026-09-16；本次範圍修訂沿用下列參考，未重新查核交易所或供應商資料。型別、責任分配與核心先行的範圍是設計決策，不是交易所規格原文。

- [R1] [TWSE 集中市場交易制度介紹](https://www.twse.com.tw/zh/products/system/trading.html)：一般交易之競價方式、暫緩開收盤與盤中價穩。頁面同時含沿革，歷史條文不可混當現行規則。
- [R2] [TWSE：處置、分盤交易等有價證券新增資訊揭露](https://shl.twse.com.tw/newsArticle/detail/4028e4f68c677ebf018c67ad618c0010?tagName=)：2022-09-26 起的相關資訊揭露與價穩配套；本目標只採普通交易範圍。
- [R3] [TWSE 投資 Q&A，第 26–27 題](https://investoredu.twse.com.tw/pages/TWSE_InvestmentQA.aspx?ID=14&Page=2)：延後收盤結果與不連續再次暫緩開收盤。
- [C1] [目前 annotations.rs](https://github.com/hschi1106/osmium-lab/blob/b403ce6e5b04d67c6687ca33567ff6e02ebdb173/crates/market-types/src/annotations.rs)：既有 MatchingMethod 與交易所 raw wrappers。
- [C2] [目前 event.rs](https://github.com/hschi1106/osmium-lab/blob/b403ce6e5b04d67c6687ca33567ff6e02ebdb173/crates/market-types/src/event.rs)：現行 payload 與 IndicativeAuctionKind。
- [C3] [目前 context.rs](https://github.com/hschi1106/osmium-lab/blob/b403ce6e5b04d67c6687ca33567ff6e02ebdb173/crates/strategy-api/src/context.rs)：IndicativeReason、TWSE／TPEx evaluators、session phase。
- [C4] [目前 TWSE 介面契約](https://github.com/hschi1106/osmium-lab/blob/b403ce6e5b04d67c6687ca33567ff6e02ebdb173/docs/interfaces/twse.md)：wire bit、status sentinel、試算與正式結果、已有來源驗證的記錄。
- [C5] [目前 TPEx 介面契約](https://github.com/hschi1106/osmium-lab/blob/b403ce6e5b04d67c6687ca33567ff6e02ebdb173/docs/interfaces/tpex.md)：TPEx 自身契約與已有來源驗證記錄，不是本次重新查詢真實資料的證明。
- [C6] [目前 execution model](https://github.com/hschi1106/osmium-lab/blob/b403ce6e5b04d67c6687ca33567ff6e02ebdb173/docs/architecture/execution-model.md)：既有 subsequent-event、scheduled、auction fill 與資料可見性限制。

原版未完成全部 TPEx 現行制度與真實來源認證，本次也不補做。既有離線 fixtures 是回歸依據，不以 TWSE 規格取代 TPEx 語義。背景資料取得、新供應商 mapping、歷史規則適用性與真實資料認證留待後續目標；不得將它們列為本次必做實驗或資料抓取。

## 執行紀錄

- Baseline revision／working tree：待執行時記錄；上列 SHA 僅為設計參考。
- 已完成：本檔已收斂成中立核心與可選記憶體初始化；移除 profile loader、來源補齊與 Teralion 完整 mapping 驗收要求；尚未實作。
- 驗證命令／結果：尚未執行 Rust 測試。
- 剩餘風險：必要共用型別變更對既有來源路徑的影響、可選初始化與既有 StateField／run identity 的接法。未接入的真實背景／來源資料限制是已知範圍界線，不因此開發新 loader。
- 下一步：完成 goal-01 後，從中立 DomainEvent／核心初始化入口鎖定六種合成案例與必要回歸，再交由 Sol medium 實作；不從 Teralion payload 探索開始。
