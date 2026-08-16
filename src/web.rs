use axum::{
    extract::State,
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use std::sync::{Arc, Mutex};
use crate::db::Db;
use crate::logger::LogStore;

pub struct AppState {
    pub db: Arc<Mutex<Db>>,
    pub logger: LogStore,
}

pub async fn start_server(db: Arc<Mutex<Db>>, logger: LogStore, port: u16) {
    let state = Arc::new(AppState { db, logger });

    let app = Router::new()
        .route("/", get(dashboard_html))
        .route("/api/stats", get(get_stats))
        .route("/api/folders", get(get_folders))
        .route("/api/recent", get(get_recent))
        .route("/api/logs", get(get_logs))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .expect("Failed to bind web server port");

    println!("📊 Monitoring Dashboard available at http://localhost:{}", port);

    axum::serve(listener, app).await.expect("Web server error");
}

async fn get_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    match db.get_stats() {
        Ok(stats) => Json(stats).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_folders(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    match db.get_folder_stats() {
        Ok(folders) => Json(folders).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_recent(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let db = state.db.lock().unwrap();
    match db.get_recent_files() {
        Ok(files) => Json(files).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_logs(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state.logger.get_all()).into_response()
}

async fn dashboard_html() -> Html<&'static str> {
    Html(r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Rust Backup Monitor</title>
    <style>
        :root {
            --bg: #0f172a;
            --card-bg: #1e293b;
            --text: #f8fafc;
            --text-dim: #94a3b8;
            --accent: #38bdf8;
            --heal: #4ade80;
            --error: #f87171;
            --border: #334155;
        }

        * { box-sizing: border-box; margin: 0; padding: 0; }
        
        body { 
            font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; 
            background: var(--bg); 
            color: var(--text); 
            padding: 1rem; 
            max-width: 1200px; 
            margin: 0 auto; 
            display: flex;
            flex-direction: column;
            gap: 1.5rem;
        }

        header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding-bottom: 0.75rem;
            border-bottom: 1px solid var(--border);
        }

        h1 { font-size: 1.5rem; font-weight: 700; color: #f1f5f9; }

        .status-badge {
            background: #166534;
            color: #86efac;
            padding: 0.25rem 0.75rem;
            border-radius: 9999px;
            font-size: 0.85rem;
            font-weight: 600;
        }

        .grid { 
            display: grid; 
            grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); 
            gap: 1rem; 
        }

        .card { 
            background: var(--card-bg); 
            padding: 1.25rem; 
            border-radius: 8px; 
            border: 1px solid var(--border); 
        }

        .card h3 { margin-bottom: 0.5rem; font-size: 0.85rem; color: var(--text-dim); text-transform: uppercase; letter-spacing: 0.05em; }
        .card p { font-size: 1.6rem; font-weight: bold; color: var(--accent); word-break: break-all; }

        .section-title { font-size: 1.15rem; color: #f1f5f9; font-weight: 600; }

        .table-container {
            width: 100%;
            overflow-x: auto;
            border-radius: 8px;
            border: 1px solid var(--border);
            background: var(--card-bg);
        }

        table { width: 100%; border-collapse: collapse; min-width: 500px; }
        th, td { padding: 0.75rem 1rem; text-align: left; border-bottom: 1px solid var(--border); }
        th { background: #334155; color: #cbd5e1; font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.05em; }
        tr:last-child td { border-bottom: none; }

        .tag { background: #0284c7; color: #fff; padding: 0.2rem 0.5rem; border-radius: 4px; font-size: 0.8rem; font-weight: 600; display: inline-block; }

        /* Bottom Terminal Logs Layout */
        .terminal-container {
            background: #020617;
            border: 1px solid var(--border);
            border-radius: 8px;
            overflow: hidden;
            display: flex;
            flex-direction: column;
            margin-top: 0.5rem;
        }

        .terminal-header {
            background: #0f172a;
            padding: 0.6rem 1rem;
            font-size: 0.85rem;
            font-weight: 600;
            color: var(--text-dim);
            border-bottom: 1px solid var(--border);
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .terminal-body {
            padding: 1rem;
            max-height: 280px;
            overflow-y: auto;
            font-family: "JetBrains Mono", Consolas, Monaco, monospace;
            font-size: 0.825rem;
            display: flex;
            flex-direction: column;
            gap: 0.4rem;
        }

        .log-line { display: flex; gap: 0.75rem; word-break: break-all; }
        .log-time { color: var(--text-dim); flex-shrink: 0; }
        .log-level { font-weight: bold; min-width: 55px; flex-shrink: 0; }
        .level-INFO { color: var(--accent); }
        .level-HEAL { color: var(--heal); }
        .level-ERROR { color: var(--error); }

        @media (max-width: 600px) {
            body { padding: 0.75rem; gap: 1rem; }
            h1 { font-size: 1.25rem; }
            .card p { font-size: 1.3rem; }
            th, td { padding: 0.5rem 0.75rem; font-size: 0.85rem; }
        }
    </style>
</head>
<body>
    <header>
        <h1>🛡️ Backup Monitor</h1>
        <span class="status-badge">● Live</span>
    </header>
    
    <div class="grid">
        <div class="card"><h3>Total Indexed Files</h3><p id="total-files">0</p></div>
        <div class="card"><h3>Total Backed-Up Data</h3><p id="total-bytes">0 MB</p></div>
        <div class="card"><h3>Last Activity</h3><p id="last-sync" style="font-size:1.1rem; padding-top:0.3rem;">Never</p></div>
    </div>

    <h2 class="section-title">Watched Folder Breakdown</h2>
    <div class="table-container">
        <table>
            <thead>
                <tr><th>Folder ID</th><th>Total Files</th><th>Data Size</th></tr>
            </thead>
            <tbody id="folder-table"></tbody>
        </table>
    </div>

    <h2 class="section-title">Recently Synced Files</h2>
    <div class="table-container">
        <table>
            <thead>
                <tr><th>Folder</th><th>Relative Path</th><th>Size</th><th>Last Synced</th></tr>
            </thead>
            <tbody id="recent-table"></tbody>
        </table>
    </div>

    <div class="terminal-container">
        <div class="terminal-header">
            <span>💻 SYSTEM LOGS & ERRORS</span>
            <span id="log-count" style="font-weight: normal; font-size: 0.8rem;">0 entries</span>
        </div>
        <div class="terminal-body" id="logs-container">
            <div style="color: var(--text-dim);">Connecting to daemon...</div>
        </div>
    </div>

    <script>
        async function updateDashboard() {
            try {
                // 1. Fetch Stats
                const stats = await (await fetch('/api/stats')).json();
                document.getElementById('total-files').innerText = stats.total_files || 0;
                document.getElementById('total-bytes').innerText = ((stats.total_bytes || 0) / (1024 * 1024)).toFixed(2) + ' MB';
                document.getElementById('last-sync').innerText = stats.last_synced_timestamp 
                    ? new Date(stats.last_synced_timestamp * 1000).toLocaleString() 
                    : 'Never';

                // 2. Fetch Folders
                const folders = await (await fetch('/api/folders')).json();
                const folderTable = document.getElementById('folder-table');
                folderTable.innerHTML = folders.map(f => `
                    <tr>
                        <td><span class="tag">${escapeHtml(f.folder_id)}</span></td>
                        <td>${f.total_files} files</td>
                        <td>${(f.total_bytes / (1024 * 1024)).toFixed(2)} MB</td>
                    </tr>
                `).join('');

                // 3. Fetch Recent Files
                const recent = await (await fetch('/api/recent')).json();
                const recentTable = document.getElementById('recent-table');
                recentTable.innerHTML = recent.map(f => `
                    <tr>
                        <td><span class="tag">${escapeHtml(f.folder_id)}</span></td>
                        <td>${escapeHtml(f.relative_path)}</td>
                        <td>${(f.file_size / 1024).toFixed(1)} KB</td>
                        <td>${new Date(f.last_synced * 1000).toLocaleTimeString()}</td>
                    </tr>
                `).join('');

                // 4. Fetch Logs
                const logs = await (await fetch('/api/logs')).json();
                const logsContainer = document.getElementById('logs-container');
                document.getElementById('log-count').innerText = `${logs.length} entries`;

                if (logs.length === 0) {
                    logsContainer.innerHTML = '<div style="color: var(--text-dim);">No logs recorded yet.</div>';
                } else {
                    const isAtBottom = logsContainer.scrollHeight - logsContainer.clientHeight <= logsContainer.scrollTop + 50;

                    logsContainer.innerHTML = logs.map(log => `
                        <div class="log-line">
                            <span class="log-time">[${log.timestamp}]</span>
                            <span class="log-level level-${log.level}">[${log.level}]</span>
                            <span>${escapeHtml(log.message)}</span>
                        </div>
                    `).join('');

                    if (isAtBottom) {
                        logsContainer.scrollTop = logsContainer.scrollHeight;
                    }
                }
            } catch (err) {
                console.error("Failed updating dashboard:", err);
            }
        }

        function escapeHtml(str) {
            if (!str) return '';
            return String(str)
                .replace(/&/g, "&amp;")
                .replace(/</g, "&lt;")
                .replace(/>/g, "&gt;");
        }

        updateDashboard();
        setInterval(updateDashboard, 3000);
    </script>
</body>
</html>
    "#)
}