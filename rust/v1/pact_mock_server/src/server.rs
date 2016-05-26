use rustful::{
    Server,
    Handler,
    Context,
    Response,
    Router,
    TreeRouter
};
use rustful::StatusCode;
use rustful::header::{
    ContentType,
    AccessControlAllowOrigin,
    AccessControlAllowMethods,
    AccessControlAllowHeaders
};
use rustful::Method::{Get, Post};
use hyper::method::Method;
use hyper::mime::{Mime, TopLevel, SubLevel, Attr, Value};
use pact_matching::models::Pact;
use pact_mock_server::{
    start_mock_server,
    iterate_mock_servers,
    MockServer
};
use uuid::Uuid;
use std::error::Error;
use rustc_serialize::json::Json;
use std::borrow::Borrow;
use std::iter::FromIterator;
use verify;

fn add_cors_headers(response: &mut Response) {
    response.headers_mut().set(AccessControlAllowOrigin::Any);
    response.headers_mut().set(AccessControlAllowMethods(vec![Method::Get, Method::Post]));
    response.headers_mut().set(AccessControlAllowHeaders(vec!["Content-Type".into()]));
}

struct MasterServerHandler;

impl Handler for MasterServerHandler {
    fn handle_request(&self, context: Context, response: Response) {
        match context.method {
            Get => list_servers(response),
            Post => {
                let path = context.uri.clone();
                if path.as_utf8_path().unwrap() == "/" {
                    start_provider(context, response)
                } else {
                    verify_mock_server_request(context, response)
                }
            },
            _ => ()
        }
    }
}

fn start_provider(mut context: Context, mut response: Response) {
    add_cors_headers(&mut response);
    let json_result = context.body.read_json_body();
    match json_result {
        Ok(pact_json) => {
            let pact = Pact::from_json(&pact_json);
            let mock_server_id = Uuid::new_v4().simple().to_string();
            match start_mock_server(mock_server_id.clone(), pact) {
                Ok(mock_server) => {
                    response.set_status(StatusCode::Ok);
                    response.headers_mut().set(
                        ContentType(Mime(TopLevel::Application, SubLevel::Json,
                                         vec![(Attr::Charset, Value::Utf8)]))
                    );
                    let mock_server_json = Json::Object(btreemap!{
                        s!("id") => Json::String(mock_server_id),
                        s!("port") => Json::I64(mock_server as i64),
                    });
                    let json_response = Json::Object(btreemap!{ s!("mockServer") => mock_server_json });
                    response.send(json_response.to_string());
                },
                Err(msg) => {
                    response.set_status(StatusCode::BadRequest);
                    response.send(msg);
                }
            }
        },
        Err(err) => {
            error!("Could not parse pact json: {}", err);
            response.set_status(StatusCode::BadRequest);
            response.send(err.description());
        }
    }
}

fn list_servers(mut response: Response) {
    add_cors_headers(&mut response);
    response.set_status(StatusCode::Ok);
    response.headers_mut().set(
        ContentType(Mime(TopLevel::Application, SubLevel::Json,
                         vec![(Attr::Charset, Value::Utf8)]))
    );

    let mut mock_servers = vec![];
    iterate_mock_servers(&mut |id: &String, ms: &MockServer| {
        let mock_server_json = ms.to_json();
        mock_servers.push(mock_server_json);
    });

    let json_response = Json::Object(btreemap!{ s!("mockServers") => Json::Array(mock_servers) });
    response.send(json_response.to_string());
}

pub fn verify_mock_server_request(context: Context, mut response: Response) {
    add_cors_headers(&mut response);
    response.headers_mut().set(
        ContentType(Mime(TopLevel::Application, SubLevel::Json,
                         vec![(Attr::Charset, Value::Utf8)]))
    );

    let id = context.variables.get("id").unwrap();
    match verify::validate_id(id.borrow()) {
        Ok(ms) => {
            let mut map = btreemap!{ s!("mockServer") => ms.to_json() };
            let mismatches = ms.mismatches();
            if !mismatches.is_empty() {
                response.set_status(StatusCode::BadRequest);
                map.insert(s!("mismatches"), Json::Array(
                    Vec::from_iter(mismatches.iter()
                        .map(|m| m.to_json()))));
            } else {
                response.set_status(StatusCode::Ok);
            }

            let json_response = Json::Object(map);
            response.send(json_response.to_string());
        },
        Err(err) => {
            response.set_status(StatusCode::UnprocessableEntity);
            response.send(Json::String(err).to_string());
        }
    }
}

pub fn start_server(port: u16) -> Result<(), i32> {
    let router = insert_routes! {
        TreeRouter::new() => {
            "/" => {
                Get: MasterServerHandler,
                Post: MasterServerHandler
            },
            "/mockserver/:id/verify" => Post: MasterServerHandler
        }
    };

    let server_result = Server {
        handlers: router,
        host: port.into(),
        ..Server::default()
    }.run();

    match server_result {
        Ok(server) => {
            info!("Server started on port {}", server.socket.port());
            Ok(())
        },
        Err(e) => {
            error!("could not start server: {}", e);
            Err(1)
        }
    }
}