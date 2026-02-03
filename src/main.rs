use chrono::Local;
use mime_guess::from_path;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Component, Path, PathBuf};
use tiny_http::{Header, Request, Response, Server, StatusCode};

fn main() {
    let mut args = env::args_os().skip(1);
    let base_dir = match args.next() {
        Some(arg) => PathBuf::from(arg),
        None => env::current_dir().expect("failed to get current dir"),
    };

    if args.next().is_some() {
        eprintln!("Usage: srvplz [directory]");
        std::process::exit(2);
    }

    let base_dir = match fs::canonicalize(&base_dir) {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("Invalid directory: {err}");
            std::process::exit(2);
        }
    };

    if !base_dir.is_dir() {
        eprintln!("Not a directory: {}", base_dir.display());
        std::process::exit(2);
    }

    let base_port = 8000u16;
    let max_retries = 25u16;
    let (server, port) = bind_server(base_port, max_retries);

    println!("Serving HTTP on :: port {port} (http://[::]:{port}/) ...");

    for request in server.incoming_requests() {
        handle_request(request, &base_dir);
    }
}

fn bind_server(base_port: u16, max_retries: u16) -> (Server, u16) {
    for offset in 0..=max_retries {
        let port = base_port + offset;
        let addr = format!("[::]:{port}");
        match Server::http(&addr) {
            Ok(server) => return (server, port),
            Err(err) => {
                let addr_in_use = is_addr_in_use(err.as_ref());
                if addr_in_use && offset < max_retries {
                    continue;
                }

                if addr_in_use {
                    eprintln!(
                        "Failed to bind any port in range {}-{} after {max_retries} retries.",
                        base_port, port
                    );
                } else {
                    eprintln!("Failed to bind {addr}: {err}");
                }
                std::process::exit(1);
            }
        }
    }

    eprintln!(
        "Failed to bind any port in range {}-{} after {max_retries} retries.",
        base_port,
        base_port + max_retries
    );
    std::process::exit(1);
}

fn is_addr_in_use(err: &(dyn Error + 'static)) -> bool {
    let mut current: Option<&(dyn Error + 'static)> = Some(err);
    while let Some(error) = current {
        if let Some(io_err) = error.downcast_ref::<std::io::Error>() {
            if io_err.kind() == std::io::ErrorKind::AddrInUse {
                return true;
            }
        }
        current = error.source();
    }
    false
}

fn handle_request(request: Request, base_dir: &Path) {
    let method = request.method().as_str().to_string();
    let url = request.url().to_string();
    let (path, _query) = url.split_once('?').unwrap_or((url.as_str(), ""));
    let version = (request.http_version().0, request.http_version().1);
    let remote_addr = request
        .remote_addr()
        .map(|addr| addr.ip().to_string())
        .unwrap_or_else(|| "-".to_string());

    let (status, response) = match method.as_str() {
        "GET" | "HEAD" => route_request(path, &method, base_dir),
        _ => {
            let mut response = response_with_status(405, &method, "Method Not Allowed");
            response.add_header(header("Allow", "GET, HEAD"));
            (405, response)
        }
    };

    let _ = request.respond(response);
    log_request(&remote_addr, &method, path, version, status);
}

fn route_request(
    path: &str,
    method: &str,
    base_dir: &Path,
) -> (u16, Response<std::io::Cursor<Vec<u8>>>) {
    if path.len() > 1 && path.ends_with('/') {
        let location = path.trim_end_matches('/');
        let mut response = Response::from_data(Vec::new()).with_status_code(StatusCode(301));
        response.add_header(header("Location", location));
        return (301, response);
    }

    let decoded = match urlencoding::decode(path) {
        Ok(value) => value,
        Err(_) => return (400, response_with_status(400, method, "Bad Request")),
    };

    let target = if decoded == "/" {
        base_dir.join("index.html")
    } else {
        let relative = &decoded[1..];
        match sanitized_relative_path(relative) {
            Some(rel) => base_dir.join(rel),
            None => return (404, response_with_status(404, method, "Not Found")),
        }
    };

    let target = match fs::metadata(&target) {
        Ok(meta) if meta.is_dir() => {
            let index = target.join("index.html");
            if index.is_file() {
                index
            } else {
                return (404, response_with_status(404, method, "Not Found"));
            }
        }
        Ok(_) => target,
        Err(_) => return (404, response_with_status(404, method, "Not Found")),
    };

    match build_file_response(&target, method) {
        Ok(response) => (200, response),
        Err(_) => (
            500,
            response_with_status(500, method, "Internal Server Error"),
        ),
    }
}

fn sanitized_relative_path(path: &str) -> Option<PathBuf> {
    let rel = Path::new(path);
    for component in rel.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => return None,
        }
    }
    Some(rel.to_path_buf())
}

fn build_file_response(
    path: &Path,
    method: &str,
) -> std::io::Result<Response<std::io::Cursor<Vec<u8>>>> {
    let mime = from_path(path).first_or_octet_stream();
    let content_type = header("Content-Type", mime.essence_str());
    let len = fs::metadata(path)?.len();

    let mut response = if method == "HEAD" {
        Response::from_data(Vec::new()).with_status_code(StatusCode(200))
    } else {
        let body = fs::read(path)?;
        Response::from_data(body).with_status_code(StatusCode(200))
    };

    response.add_header(content_type);
    if method == "HEAD" {
        response.add_header(header("Content-Length", len.to_string()));
    }
    Ok(response)
}

fn response_with_status(
    status: u16,
    method: &str,
    body: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
    if method == "HEAD" {
        Response::from_data(Vec::new()).with_status_code(StatusCode(status))
    } else {
        Response::from_string(body).with_status_code(StatusCode(status))
    }
}

fn header(name: &str, value: impl AsRef<str>) -> Header {
    Header::from_bytes(name, value.as_ref()).expect("invalid header")
}

fn log_request(remote: &str, method: &str, path: &str, version: (u8, u8), status: u16) {
    let timestamp = Local::now().format("%d/%b/%Y %H:%M:%S");
    println!(
        "{} - - [{}] \"{} {} HTTP/{}.{}\" {} -",
        remote, timestamp, method, path, version.0, version.1, status
    );
}
