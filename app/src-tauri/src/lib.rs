use banime_core::{AppPaths, db, home};
use tauri::Manager;
use tracing_subscriber::EnvFilter;

struct AppState {
    db: db::Db,
    #[allow(dead_code)]
    paths: AppPaths,
}

#[tauri::command]
async fn get_home(state: tauri::State<'_, AppState>) -> Result<home::HomeData, String> {
    home::load(&state.db).await.map_err(|e| e.to_string())
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let paths = AppPaths::resolve(None)?;
            paths.ensure()?;
            tracing::info!(data_dir = %paths.root.display(), "starting B-Anime");
            let db = tauri::async_runtime::block_on(db::open(&paths.db))?;
            app.manage(AppState { db, paths });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_home])
        .run(tauri::generate_context!())
        .expect("error while running B-Anime");
}
