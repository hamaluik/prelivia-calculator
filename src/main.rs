use actix_web::{http::header, middleware::Logger, rt, web, App, HttpResponse, HttpServer};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ReportRequest {
    facility_name: String,
    facility_type: String,
    patients_per_day: usize,
    length_of_stay: usize,
    incidence: f64,
    email: String,
}

async fn handle_response_log(
    tx: web::Data<std::sync::mpsc::Sender<ReportRequest>>,
    params: web::Form<ReportRequest>,
) -> actix_web::Result<HttpResponse> {
    if let Err(e) = tx.send(params.into_inner()) {
        log::error!("Failed to send response for recording: {e:#}");
    }

    Ok(HttpResponse::SeeOther()
        .append_header((header::LOCATION, "/responded.html"))
        .finish())
}

fn main() -> Result<()> {
    dotenv::dotenv().ok();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "info"
        },
    ))
    .init();

    let bind = std::env::var("BIND").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = {
        use std::str::FromStr;
        let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
        let port =
            u16::from_str(&port).with_context(|| format!("Failed to parse port {port} as u16!"))?;
        port
    };
    let response_log: PathBuf = std::env::var("RESPONSELOG")
        .unwrap_or_else(|_| "responses.csv".to_string())
        .into();
    if response_log.exists() && !response_log.is_file() {
        return Err(anyhow::anyhow!(
            "Response log {} isn't a file!",
            response_log.display()
        ));
    }

    log::info!(
        "starting HTTP server on http://{bind}:{port} with responses logging to {}",
        response_log.display()
    );

    let (tx, rx) = std::sync::mpsc::channel::<ReportRequest>();

    let server = HttpServer::new(move || {
        App::new()
            .wrap(Logger::default())
            .app_data(web::Data::new(tx.clone()))
            .service(web::resource("/respond").route(web::post().to(handle_response_log)))
            .service(
                actix_files::Files::new("/", "./public")
                    .index_file("index.html")
                    .use_etag(true)
                    .use_last_modified(true)
                    .prefer_utf8(true),
            )
    })
    .bind(("127.0.0.1", 8080))?
    .run();

    let log_file_existed: bool = response_log.exists();
    let log_file = std::fs::File::options()
        .append(true)
        .write(true)
        .create(true)
        .open(&response_log)
        .with_context(|| format!("Failed to open {} for writing!", response_log.display()))?;
    std::thread::spawn(move || {
        let mut wtr = csv::WriterBuilder::new()
            .has_headers(!log_file_existed)
            .flexible(false)
            .from_writer(log_file);

        loop {
            match rx.recv() {
                Ok(req) => {
                    log::debug!("Received response: {req:?}");
                    if let Err(e) = wtr.serialize(&req) {
                        log::error!("Failed to write record {req:#?}: {e:#}");
                    }
                }
                Err(_) => {
                    log::warn!("Receiver disconnected!");
                    break;
                }
            }
        }
    });

    rt::System::new()
        .block_on(server)
        .with_context(|| format!("Failed to run HTTP server on {bind}:{port}"))?;

    Ok(())
}
