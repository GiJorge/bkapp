use axum::{
    extract::State,
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use std::sync::{Arc, Mutex};
use crate::db::Db;

pub struct AppState {
    pub db: Arc<Mutex<Db>>,
}

pub async fn start_server(db: Arc<Mutex<Db>>, port: u16) {
    let state = Arc::new(AppState { db });

    let app = Router::new()
        .route("/", get(dashboard_html))
        .route("/api/stats", get(get_stats))
        .route("/api/folders", get(get_folders))
        .route("/api/recent", get(get_recent))
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

async fn dashboard_html() -> Html<&'static str> {
    Html(r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <title>Rust Backup Monitor</title>
    <style>
        body { font-family: system-ui, sans-serif; background: #0f172a; color: #f8fafc; padding: 2rem; max-width: 1100px; margin: 0 auto; }
        .grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 1rem; margin-bottom: 2rem; }
        .card { background: #1e293b; padding: 1.5rem; border-radius: 8px; border: 1px solid #334155; }
        .card h3 { margin: 0 0 0.5rem 0; font-size: 0.9rem; color: #94a3b8; }
        .card p { margin: 0; font-size: 1.8rem; font-weight: bold; color: #38bdf8; }
        .section-title { margin-top: 2rem; margin-bottom: 1rem; font-size: 1.25rem; color: #f1f5f9; }
        table { width: 100%; border-collapse: collapse; background: #1e293b; border-radius: 8px; overflow: hidden; margin-bottom: 2rem; }
        th, td { padding: 0.75rem 1rem; text-align: left; border-bottom: 1px solid #334155; }
        th { background: #334155; color: #cbd5e1; font-size: 0.85rem; text-transform: uppercase; letter-spacing: 0.05em; }
        .tag { background: #0284c7; color: #fff; padding: 0.2rem 0.5rem; border-radius: 4px; font-size: 0.8rem; font-weight: 600; display: inline-block; }
    </style>
</head>
<body>
    <h1>🛡️ Multi-Folder Backup Monitor</h1>
    
    <div class="grid">
        <div class="card"><h3>Total Indexed Files</h3><p id="total-files">0</p></div>
        <div class="card"><h3>Total Backed-Up Data</h3><p id="total-bytes">0 MB</p></div>
        <div class="card"><h3>Last Activity</h3><p id="last-sync" style="font-size:1.1rem; padding-top:0.4rem;">Never</p></div>
    </div>

    <h2 class="section-title">Watched Folder Breakdown</h2>
    <table>
        <thead>
            <tr><th>Folder ID</th><th>Total Files</th><th>Data Size</th></tr>
        </thead>
        <tbody id="folder-table"></tbody>
    </table>

    <h2 class="section-title">Recently Synced Files</h2>
    <table>
        <thead>
            <tr><th>Folder</th><th>Relative Path</th><th>Size</th><th>Last Synced</th></tr>
        </thead>
        <tbody id="recent-table"></tbody>
    </table>

    <script>
        async function updateDashboard() {
            const stats = await (await fetch('/api/stats')).json();
            document.getElementById('total-files').innerText = stats.total_files;
            document.getElementById('total-bytes').innerText = (stats.total_bytes / (1024 * 1024)).toFixed(2) + ' MB';
            document.getElementById('last-sync').innerText = stats.last_synced_timestamp 
                ? new Date(stats.last_synced_timestamp * 1000).toLocaleString() 
                : 'Never';

            const folders = await (await fetch('/api/folders')).json();
            const folderTable = document.getElementById('folder-table');
            folderTable.innerHTML = folders.map(f => `
                <tr>
                    <td><span class="tag">${f.folder_id}</span></td>
                    <td>${f.total_files} files</td>
                    <td>${(f.total_bytes / (1024 * 1024)).toFixed(2)} MB</td>
                </tr>
            `).join('');

            const recent = await (await fetch('/api/recent')).json();
            const recentTable = document.getElementById('recent-table');
            recentTable.innerHTML = recent.map(f => `
                <tr>
                    <td><span class="tag">${f.folder_id}</span></td>
                    <td>${f.relative_path}</td>
                    <td>${(f.file_size / 1024).toFixed(1)} KB</td>
                    <td>${new Date(f.last_synced * 1000).toLocaleTimeString()}</td>
                </tr>
            `).join('');
        }

        updateDashboard();
        setInterval(updateDashboard, 3000);
    </script>
</body>
</html>
    "#)
}