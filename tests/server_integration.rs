use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use std::process::{Child, Command};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

/// Matches a GitHub contents PUT whose base64-decoded `content` field contains all given strings.
struct FileContentContains(Vec<&'static str>);

impl Match for FileContentContains {
    fn matches(&self, request: &Request) -> bool {
        let Ok(body) = serde_json::from_slice::<serde_json::Value>(&request.body) else {
            return false;
        };
        let Some(encoded) = body["content"].as_str() else {
            return false;
        };
        // GitHub strips newlines from base64; put them back before decoding
        let cleaned = encoded.replace('\n', "");
        let Ok(decoded) = B64.decode(cleaned) else {
            return false;
        };
        let text = String::from_utf8_lossy(&decoded);
        self.0.iter().all(|s| text.contains(*s))
    }
}

const OWNER: &str = "test-owner";
const REPO: &str = "test-repo";
const CONTENT_PATH: &str =
    "/repos/test-owner/test-repo/contents/content/blog/test-post/comments.yaml";

// Find a free local port. There is a small TOCTOU window, but it is fine for tests.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

async fn start_server(server_port: u16, github_mock_uri: &str) -> ServerGuard {
    let config = format!(
        r#"
listen: "127.0.0.1:{server_port}"
token: "test-token"
owner: "{OWNER}"
repo: "{REPO}"
content_dir: "content"
github_api_url: "{github_mock_uri}"
rate_limit_secs: 1
committer:
  name: "Test Bot"
  email: "bot@example.com"
"#
    );

    let config_path = std::env::temp_dir().join(format!("laberbox-test-{server_port}.yaml"));
    std::fs::write(&config_path, config).unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_laberbox"))
        .env("LABERBOX_CONFIG", &config_path)
        .spawn()
        .expect("failed to spawn laberbox binary");

    // Poll until the server accepts connections (up to 5 s)
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if client
            .post(format!("http://127.0.0.1:{server_port}"))
            .send()
            .await
            .is_ok()
        {
            break;
        }
    }

    ServerGuard(child)
}

// ── Shared mock response builders ────────────────────────────────────────────

fn ref_response() -> serde_json::Value {
    serde_json::json!({
        "ref": "refs/heads/master",
        "node_id": "ABC",
        "url": "http://localhost",
        "object": {
            "type": "commit",
            "sha": "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef",
            "url": "http://localhost"
        }
    })
}

fn file_update_response() -> serde_json::Value {
    serde_json::json!({
        "content": {
            "name": "comments.yaml",
            "path": "content/blog/test-post/comments.yaml",
            "sha": "newsha456",
            "size": 100,
            "url": "http://localhost",
            "html_url": "http://localhost",
            "git_url": "http://localhost",
            "download_url": null,
            "type": "file",
            "_links": { "self": "http://localhost", "git": "http://localhost", "html": "http://localhost" }
        },
        "commit": {
            "sha": "commitsha789",
            "url": "http://localhost",
            "html_url": "http://localhost",
            "message": "Added comment",
            "author":    { "name": "Test Bot", "email": "bot@example.com", "date": "2024-01-01T00:00:00Z" },
            "committer": { "name": "Test Bot", "email": "bot@example.com", "date": "2024-01-01T00:00:00Z" },
            "tree":    { "sha": "treesha", "url": "http://localhost" },
            "parents": []
        }
    })
}

fn pull_request_response() -> serde_json::Value {
    serde_json::json!({
        "url": "http://localhost/repos/test-owner/test-repo/pulls/1",
        "id": 1,
        "node_id": "PR_1",
        "html_url": "http://localhost/test-owner/test-repo/pull/1",
        "number": 1,
        "state": "open",
        "locked": false,
        "title": "New comment from Test User",
        "head": { "label": "test-owner:comments/test", "ref": "comments/test", "sha": "deadbeef", "user": null, "repo": null },
        "base": { "label": "test-owner:master",         "ref": "master",         "sha": "deadbeef", "user": null, "repo": null },
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z"
    })
}

/// Mount the mocks that every comment submission requires.
async fn mount_common_mocks(mock: &MockServer) {
    Mock::given(method("GET"))
        .and(path(format!("/repos/{OWNER}/{REPO}/git/ref/heads/master")))
        .respond_with(ResponseTemplate::new(200).set_body_json(ref_response()))
        .expect(1)
        .mount(mock)
        .await;

    Mock::given(method("POST"))
        .and(path(format!("/repos/{OWNER}/{REPO}/git/refs")))
        .respond_with(ResponseTemplate::new(201).set_body_json(ref_response()))
        .expect(1)
        .mount(mock)
        .await;

    Mock::given(method("POST"))
        .and(path(format!("/repos/{OWNER}/{REPO}/pulls")))
        .respond_with(ResponseTemplate::new(201).set_body_json(pull_request_response()))
        .expect(1)
        .mount(mock)
        .await;
}

async fn post_comment(port: u16) -> reqwest::Response {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .post(format!("http://127.0.0.1:{port}"))
        .form(&[
            ("path", "/blog/test-post/"),
            ("name", "Test User"),
            ("url", "https://example.com"),
            ("message", "Great post!"),
            ("redirect_url", "http://myblog.com/blog/test-post/#comments"),
        ])
        .send()
        .await
        .unwrap()
}

fn assert_redirect(response: &reqwest::Response) {
    assert_eq!(response.status(), 303, "expected redirect");
    assert_eq!(
        response.headers()["location"],
        "http://myblog.com/blog/test-post/#comments",
        "redirect should point back to the post"
    );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// First comment on a post: comments.yaml does not exist yet → server creates it.
#[tokio::test]
async fn comment_creates_new_file() {
    let github_mock = MockServer::start().await;
    mount_common_mocks(&github_mock).await;

    Mock::given(method("GET"))
        .and(path(CONTENT_PATH))
        .respond_with(
            ResponseTemplate::new(404).set_body_json(serde_json::json!({"message": "Not Found"})),
        )
        .expect(1)
        .mount(&github_mock)
        .await;

    Mock::given(method("PUT"))
        .and(path(CONTENT_PATH))
        .respond_with(ResponseTemplate::new(201).set_body_json(file_update_response()))
        .expect(1)
        .mount(&github_mock)
        .await;

    let port = free_port();
    let _server = start_server(port, &github_mock.uri()).await;

    let response = post_comment(port).await;
    assert_redirect(&response);
    // wiremock verifies expect() counts on drop
}

/// Second comment on a post: comments.yaml already exists → server appends and updates it.
#[tokio::test]
async fn comment_appends_to_existing_file() {
    let github_mock = MockServer::start().await;
    mount_common_mocks(&github_mock).await;

    let existing_yaml =
        "- id: \"old_comment\"\n  message: First!\n  name: Alice\n  url: ''\n  date: 1000\n";
    let encoded = B64.encode(existing_yaml);

    Mock::given(method("GET"))
        .and(path(CONTENT_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "type": "file",
            "encoding": "base64",
            "size": existing_yaml.len(),
            "name": "comments.yaml",
            "path": "content/blog/test-post/comments.yaml",
            "content": encoded,
            "sha": "existingsha123",
            "url": "http://localhost",
            "git_url": "http://localhost",
            "html_url": "http://localhost",
            "download_url": null,
            "_links": { "self": "http://localhost", "git": "http://localhost", "html": "http://localhost" }
        })))
        .expect(1)
        .mount(&github_mock)
        .await;

    Mock::given(method("PUT"))
        .and(path(CONTENT_PATH))
        .and(FileContentContains(vec!["First!", "Great post!"]))
        .respond_with(ResponseTemplate::new(200).set_body_json(file_update_response()))
        .expect(1)
        .mount(&github_mock)
        .await;

    let port = free_port();
    let _server = start_server(port, &github_mock.uri()).await;

    let response = post_comment(port).await;
    assert_redirect(&response);
}
