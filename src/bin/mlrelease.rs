//! Small API-only client for the local release control plane.

use std::{
    env,
    fmt::Write as _,
    fs,
    io::{Read, Write},
    net::TcpStream,
};

use serde_json::{Value, json};

fn main() {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    if let Err(error) = run(&arguments) {
        eprintln!("mlrelease: {error}");
        std::process::exit(1);
    }
}

fn run(arguments: &[String]) -> Result<(), String> {
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err(usage());
    };
    let base_url =
        env::var("ML_RELEASE_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8000".to_owned());
    let output = match command {
        "deploy" => deploy(&base_url, &arguments[1..])?,
        "status" => get(&base_url, &required_argument(arguments, 1, "release-id")?)?,
        "history" => get(
            &base_url,
            &format!(
                "/models/{}/history",
                required_argument(arguments, 1, "model")?
            ),
        )?,
        "current" => get(
            &base_url,
            &format!(
                "/models/{}/current",
                required_argument(arguments, 1, "model")?
            ),
        )?,
        _ => return Err(usage()),
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&output).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn deploy(base_url: &str, arguments: &[String]) -> Result<Value, String> {
    let reference = required_argument(arguments, 0, "model:version")?;
    let (model_name, version) = reference
        .rsplit_once(':')
        .filter(|(model, version)| !model.is_empty() && !version.is_empty())
        .ok_or_else(|| "deploy requires a model:version reference".to_owned())?;
    let image = flag_value(arguments, "--image")?;
    let metrics = read_json_file(&flag_value(arguments, "--metrics-file")?)?;
    let policy = read_json_file(&flag_value(arguments, "--policy-file")?)?;
    let create = request(
        base_url,
        "POST",
        "/releases",
        Some(json!({
            "model_name": model_name, "version": version, "image_uri": image,
        })),
    )?;
    let release_id = create
        .get("release_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "API did not return release_id".to_owned())?;
    request(
        base_url,
        "POST",
        &format!("/releases/{release_id}/evaluate"),
        Some(json!({
            "metrics": metrics, "policy": policy,
        })),
    )?;
    request(
        base_url,
        "POST",
        &format!("/releases/{release_id}/deploy"),
        None,
    )?;
    request(
        base_url,
        "POST",
        &format!("/releases/{release_id}/verify"),
        None,
    )
}

fn get(base_url: &str, path: &str) -> Result<Value, String> {
    request(base_url, "GET", path, None)
}

fn request(base_url: &str, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let (host, port, prefix) = parse_base_url(base_url)?;
    let payload = body
        .map(|value| serde_json::to_string(&value).map_err(|error| error.to_string()))
        .transpose()?;
    let request_path = format!("{}{}", prefix.trim_end_matches('/'), path);
    let mut stream = TcpStream::connect((host.as_str(), port))
        .map_err(|error| format!("cannot connect to API: {error}"))?;
    let mut request = format!(
        "{method} {request_path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nAccept: application/json\r\n"
    );
    if let Some(payload) = &payload {
        write!(
            request,
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            payload.len()
        )
        .map_err(|error| format!("cannot format request: {error}"))?;
    }
    request.push_str("\r\n");
    if let Some(payload) = payload {
        request.push_str(&payload);
    }
    stream
        .write_all(request.as_bytes())
        .map_err(|error| error.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| error.to_string())?;
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| "invalid HTTP response".to_owned())?;
    let status = head
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "invalid HTTP status".to_owned())?
        .parse::<u16>()
        .map_err(|error| error.to_string())?;
    let value: Value =
        serde_json::from_str(body).map_err(|error| format!("invalid JSON response: {error}"))?;
    if !(200..300).contains(&status) {
        return Err(value
            .get("detail")
            .and_then(Value::as_str)
            .unwrap_or("API request failed")
            .to_owned());
    }
    Ok(value)
}

fn parse_base_url(base_url: &str) -> Result<(String, u16, String), String> {
    let authority_and_path = base_url
        .strip_prefix("http://")
        .ok_or_else(|| "ML_RELEASE_API_URL must use http://".to_owned())?;
    let (authority, prefix) = authority_and_path
        .split_once('/')
        .map_or((authority_and_path, ""), |(authority, path)| {
            (authority, path)
        });
    let (host, port) =
        authority
            .rsplit_once(':')
            .map_or(Ok((authority.to_owned(), 80)), |(host, port)| {
                port.parse::<u16>()
                    .map(|port| (host.to_owned(), port))
                    .map_err(|_| "invalid API port".to_owned())
            })?;
    if host.is_empty() {
        return Err("invalid API host".to_owned());
    }
    Ok((host, port, format!("/{prefix}")))
}

fn required_argument(arguments: &[String], index: usize, name: &str) -> Result<String, String> {
    arguments
        .get(index)
        .cloned()
        .ok_or_else(|| format!("missing {name}: {}", usage()))
}

fn flag_value(arguments: &[String], flag: &str) -> Result<String, String> {
    arguments
        .iter()
        .position(|argument| argument == flag)
        .and_then(|index| arguments.get(index + 1))
        .cloned()
        .ok_or_else(|| format!("deploy requires {flag}"))
}

fn read_json_file(path: &str) -> Result<Value, String> {
    let content =
        fs::read_to_string(path).map_err(|error| format!("cannot read '{path}': {error}"))?;
    serde_json::from_str(&content).map_err(|error| format!("invalid JSON in '{path}': {error}"))
}

fn usage() -> String {
    "usage: mlrelease deploy <model>:<version> --image <image> --metrics-file <json> --policy-file <json> | status <release-id> | history <model> | current <model>".to_owned()
}
