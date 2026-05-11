# Rust Process Watcher (System Monitor & Alerting Tool)

## Project Overview
The goal is to build a high-performance system monitoring tool (TUI) that allows users to observe system processes and set custom triggers (alerts) based on resource usage. 
## Milestones

### 1. Data Ingestion Layer
- Implement a background loop that samples system state (CPU, RAM, Processes).
- Decouple data collection from data presentation.
- **Challenge:** Efficiently updating a snapshot of 100+ processes without unnecessary allocations.

### 2. Monitoring Engine (The Core)
- Implement a mechanism to "subscribe" to a specific process (by PID or Name).
- Track historical data for subscribed processes to detect trends.
- **Challenge:** Managing the lifecycle of "watched" targets when processes exit or restart.

### 3. Rule & Trigger System
- Create a logic engine to handle alerts.
- Example: `IF process "node" RAM > 2GB FOR > 30s THEN trigger alert`.
- **Challenge:** Designing a flexible rule system that doesn't leak memory or cause data races.

### 4. Interactive TUI (Terminal User Interface)
- Build a dynamic dashboard using `ratatui`.
- Features: Sortable process table, process selection for watching, and an alert log.
- **Challenge:** Handling keyboard input and UI rendering concurrently with background data sampling.

### 5. Persistence (Optional)
- Save and load "watch lists" and alert configurations via a config file (e.g., TOML/YAML).

## Technical Requirements & "Struggle" Points

- **Zero Panic Policy:** Handle all system errors (I/O, missing PIDs, terminal resizing) gracefully using `Result` and `Option`. No `.unwrap()`.
- **Async Concurrency:** Use `tokio` for the background sampling to ensure the UI remains responsive at all times.
- **Efficient Ownership:** Share process data between the collector and the UI using thread-safe primitives (`Arc`, `RwLock`, or Channels) without cloning the entire process list every frame.
- **Performance:** The watcher itself should have a negligible footprint (<1% CPU).

## Suggested Stack
- `sysinfo`: System data retrieval.
- `ratatui`: TUI framework.
- `tokio`: Async runtime.
- `crossterm`: Terminal backend and event handling.
- `anyhow` / `thiserror`: Idiomatic error handling.

## Definition of Done
A tool that runs in the terminal, shows a live list of processes, allows you to select one to "watch", and triggers a visual notification or log entry when that process exceeds a defined resource threshold.
