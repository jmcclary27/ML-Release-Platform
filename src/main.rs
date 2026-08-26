use ml_release_platform::{config::Settings, create_app};

#[tokio::main]
async fn main() {
    let settings = Settings::from_environment();
    let app = create_app(Some(&settings.database_url))
        .await
        .expect("unable to initialise the application database");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8000")
        .await
        .expect("unable to bind HTTP listener");
    axum::serve(listener, app)
        .await
        .expect("HTTP server terminated unexpectedly");
}
