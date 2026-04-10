use std::fs;
use std::path::Path;
use std::process::Command;

/// Copies the canonical shortcode into the test Zola project before each run
/// so the test always exercises the current version of the template.
fn install_shortcode(zola_root: &Path) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("integrations/zola/templates/shortcodes/comments.html");
    let dst = zola_root.join("templates/shortcodes/comments.html");
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    fs::copy(&src, &dst).expect("failed to copy shortcode into test project");
}

fn zola_available() -> bool {
    Command::new("zola")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn zola_renders_comments() {
    if !zola_available() {
        eprintln!("zola not found in PATH — skipping test");
        return;
    }

    let zola_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/zola");
    let output_dir = zola_root.join("public");

    install_shortcode(&zola_root);

    // Clean any previous build output
    let _ = fs::remove_dir_all(&output_dir);

    let result = Command::new("zola")
        .args(["build", "--output-dir", "public"])
        .current_dir(&zola_root)
        .output()
        .expect("failed to run zola");

    assert!(
        result.status.success(),
        "zola build failed:\n{}",
        String::from_utf8_lossy(&result.stderr),
    );

    // --- Post with comments ---
    let with_html = fs::read_to_string(output_dir.join("with-comments/index.html"))
        .expect("with-comments page missing from output");

    // Existing comments are rendered
    assert!(with_html.contains("Alice"), "comment author 'Alice' not found");
    assert!(
        with_html.contains("Hello from the test suite"),
        "comment message not found"
    );
    // Commenter with a URL gets a link with the full URL intact
    assert!(
        with_html.contains("https://alice.example.com"),
        "comment URL not rendered"
    );
    // Bob has no URL — his name should appear as plain text, no anchor
    assert!(with_html.contains("Bob"), "comment author 'Bob' not found");
    assert!(
        !with_html.contains("href=\"\""),
        "empty href rendered for URL-less comment"
    );
    // Form is present
    assert!(
        with_html.contains("Submit comment"),
        "submit button not found"
    );
    // Hidden path field matches page path
    assert!(
        with_html.contains(r#"name="path" value="/with-comments/""#),
        "hidden path field has wrong value"
    );
    // Form action points to configured server URL
    assert!(
        with_html.contains(r#"action="http://localhost:3000""#),
        "form action does not point to comment server"
    );

    // --- Post without comments ---
    let without_html = fs::read_to_string(output_dir.join("without-comments/index.html"))
        .expect("without-comments page missing from output");

    assert!(
        without_html.contains("No comments yet"),
        "'No comments yet' message not found"
    );
    assert!(
        without_html.contains("Submit comment"),
        "submit button not found on comment-free page"
    );
    assert!(
        without_html.contains(r#"name="path" value="/without-comments/""#),
        "hidden path field has wrong value on comment-free page"
    );

    // Clean up build output
    let _ = fs::remove_dir_all(&output_dir);
}
