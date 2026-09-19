# NNN：目標名稱

Status: pending
Depends on: <前置 goal 或 none>

## 目標

用一小段話描述完成後少掉的複雜度、得到的能力或更清楚的責任邊界。

## 問題與執行前證據

- 實作前重新確認目前 `file::symbol` / data flow。
- goal 中寫到的舊路徑只是定位提示，不是現況保證。
- 未證實問題標成候選，不可直接重構。

## 範圍

### 必須做

- ...

### 不做

- ...

### 必須保持

- domain correctness / product capability ...

## LOC

開始與結束均依 `README.md` 使用 `~/cloc/cloc`。

```text
Rust LOC before:
Rust LOC after:
delta:
```

LOC 不是唯一驗收；說明刪掉的概念、路徑或 duplication。

## 驗收

- [ ] 主要問題已移除或有具體證據說明不應改。
- [ ] 新契約只保留一條 production path。
- [ ] 必要 domain regression / new contract tests 通過。
- [ ] workspace fmt / test / clippy 通過。
- [ ] breaking changes 與 version / checksum 影響有說明。
- [ ] 文件與實作一致。
- [ ] after-cloc 已記錄。

## 執行紀錄

- Baseline revision / working tree：
- Rust LOC before：
- 決定與已完成：
- 驗證命令 / 結果：
- Rust LOC after / delta：
- Breaking changes：
- 阻塞 / 剩餘風險：
- 下一步：
