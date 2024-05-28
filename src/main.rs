
use actix_files as fs;
use actix_multipart::Multipart;
use actix_web::{web, App, HttpResponse, HttpServer, Result};
use futures_util::stream::StreamExt as _;
use std::collections::HashMap;
use std::io::Write;
use actix_web::web::Data;
use rusqlite::{params, Connection, Result as SqlResult};
use std::sync::Mutex;
use rand::{distributions::Alphanumeric, Rng};

// Maximum file size (20 MB)
const MAX_SIZE: usize = 20 * 1024 * 1024;
const POSTS_PER_PAGE: usize = 30;

async fn save_file(mut payload: Multipart, conn: web::Data<Mutex<Connection>>) -> Result<HttpResponse> {
    let mut title = String::new();
    let mut message = String::new();
    let mut file_path = None;
    let mut parent_id: i32 = 0;

    while let Some(item) = payload.next().await {
        let mut field = item?;
        let content_disposition = field.content_disposition().clone();
        let name = content_disposition.get_name().unwrap_or("").to_string();

        match name.as_str() {
            "title" => {
                while let Some(chunk) = field.next().await {
                    let data = chunk?;
                    title.push_str(&String::from_utf8_lossy(&data));
                }
            },
            "message" => {
                while let Some(chunk) = field.next().await {
                    let data = chunk?;
                    message.push_str(&String::from_utf8_lossy(&data));
                }
            },
            "file" => {
                if let Some(filename) = content_disposition.get_filename() {
                    let file_extension = filename.split('.').last().unwrap_or("");
                    let sanitized_filename = sanitize_filename::sanitize(&filename);
                    let unique_id: String = rand::thread_rng()
                        .sample_iter(&Alphanumeric)
                        .take(6)
                        .map(char::from)
                        .collect();
                    let unique_filename = format!("{}-{}", unique_id, sanitized_filename);

                    let valid_image_extensions = ["jpg", "jpeg", "png", "gif", "webp"];
                    let valid_video_extensions = ["mp4", "mp3", "webm"];

                    if valid_image_extensions.contains(&file_extension) || valid_video_extensions.contains(&file_extension) {
                        let file_path_string = format!("./static/{}", unique_filename);
                        let file_path_clone = file_path_string.clone();
                        let mut f = web::block(move || std::fs::File::create(file_path_clone)).await??;

                        while let Some(chunk) = field.next().await {
                            let data = chunk?;
                            f = web::block(move || f.write_all(&data).map(|_| f)).await??;
                        }

                        file_path = Some(file_path_string);
                    }
                }
            },
            "parent_id" => {
                while let Some(chunk) = field.next().await {
                    let data = chunk?;
                    parent_id = String::from_utf8_lossy(&data).trim().parse().unwrap_or(0);
                }
            },
            _ => {},
        }
    }

    if title.trim().is_empty() || message.trim().is_empty() {
        return Ok(HttpResponse::BadRequest().body("Title and message are mandatory."));
    }

    if title.len() > 20 || message.len() > 40000 {
        return Ok(HttpResponse::BadRequest().body("Title or message is too long."));
    }

    let post_id: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .map(char::from)
        .collect();

    let conn = conn.lock().unwrap();
    conn.execute(
        "INSERT INTO files (post_id, parent_id, title, message, file_path) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![post_id, parent_id, title, message, file_path],
    ).unwrap();

    if parent_id == 0 {
        Ok(HttpResponse::SeeOther().append_header(("Location", "/")).finish())
    } else {
        Ok(HttpResponse::SeeOther().append_header(("Location", format!("/post/{}", parent_id))).finish())
    }
}

async fn view_post(conn: web::Data<Mutex<Connection>>, path: web::Path<i32>) -> Result<HttpResponse> {
    let conn = conn.lock().unwrap();
    let post_id = path.into_inner();

    let mut stmt = conn.prepare("SELECT id, post_id, parent_id, title, message, file_path FROM files WHERE id = ?1 OR parent_id = ?1 ORDER BY id ASC").unwrap();
    let posts = stmt.query_map(params![post_id], |row| {
        Ok((
            row.get::<_, i32>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i32>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    }).unwrap();

    let mut body = String::new();

    body.push_str("<html><head><title>View Post</title><style>");
    body.push_str(r#"
        body {
            background-color: #121212;
            color: #FFFFFF;
            font-family: Arial, sans-serif;
        }
        .centered-form {
            display: flex;
            justify-content: center;
            margin-bottom: 20px;
        }
        form {
            display: flex;
            flex-direction: column;
            width: 300px;
            margin-bottom: 20px;
        }
        input[type="text"] {
            width: 50%;
        }
        textarea {
            height: 150px;
        }
        .post {
            border-bottom: 5px solid #333333;
            padding: 10px 0;
        }
        img, video {
            max-width: 200px;
            max-height: 200px;
            display: block;
            margin-bottom: 10px;
        }
        .back-link {
            display: block;
            margin-bottom: 20px;
            text-align: center;
        }
        .back-link button {
            background-color: #007bff;
            color: #ffffff;
            padding: 10px;
            border: none;
            border-radius: 5px;
            cursor: pointer;
        }
        .back-link button:hover {
            background-color: #0056b3;
        }
        button {
            background-color: #007bff;
            color: #ffffff;
            padding: 10px;
            border: none;
            border-radius: 5px;
            cursor: pointer;
        }
        button:hover {
            background-color: #0056b3;
        }
    "#);
    body.push_str("</style></head><body>");
    body.push_str(r#"<div class="back-link"><a href="/"><button>Return to Main Board</button></a></div>"#);
    body.push_str(
        &format!(r#"<div class="centered-form"><form action="/upload" method="post" enctype="multipart/form-data">
            <input type="hidden" name="parent_id" value="{}">
            <input type="text" name="title" maxlength="20" placeholder="Title" required><br>
            <textarea name="message" maxlength="40000" placeholder="Message" required></textarea><br>
            <input type="file" name="file"><br>
            <button type="submit">Reply</button>
        </form></div>"#, post_id),
    );

    let mut is_original_post = true;
    let mut reply_count = 1;

    for post in posts {
        let (_id, _post_id, _parent_id, title, message, file_path) = post.unwrap();
        
        body.push_str("<div class=\"post\">");
        if is_original_post {
            body.push_str("<div class=\"post-id\">Original Post</div>");
            is_original_post = false;
        } else {
            body.push_str(&format!("<div class=\"post-id\">Reply {}</div>", reply_count));
            reply_count += 1;
        }
        body.push_str(&format!("<div class=\"post-title\">{}</div>", title));
        if let Some(file_path) = file_path {
            if file_path.ends_with(".jpg") || file_path.ends_with(".jpeg") || file_path.ends_with(".png") || file_path.ends_with(".gif") || file_path.ends_with(".webp") {
                body.push_str(&format!(r#"<img src="/static/{}"><br>"#, file_path.trim_start_matches("./static/")));
            } else if file_path.ends_with(".mp4") || file_path.ends_with(".mp3") || file_path.ends_with(".webm") {
                body.push_str(&format!(r#"<video controls><source src="/static/{}"></video><br>"#, file_path.trim_start_matches("./static/")));
            }
        }
        body.push_str(&format!("<div class=\"post-message\">{}</div>", message));
        body.push_str("</div>");
    }

    body.push_str("</body></html>");

    Ok(HttpResponse::Ok().content_type("text/html").body(body))
}

async fn index(conn: web::Data<Mutex<Connection>>, query: web::Query<HashMap<String, String>>) -> Result<HttpResponse> {
    let conn = conn.lock().unwrap();
    let page: usize = query.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let offset = (page - 1) * POSTS_PER_PAGE;

    let mut stmt = conn.prepare("SELECT id, post_id, title, message, file_path FROM files WHERE parent_id = 0 ORDER BY id DESC LIMIT ?1 OFFSET ?2").unwrap();
    let posts = stmt.query_map(params![POSTS_PER_PAGE as i64, offset as i64], |row| {
        Ok((
            row.get::<_, i32>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    }).unwrap();

    let mut body = String::new();

    body.push_str("<html><head><title>File Upload</title><style>");
    body.push_str(r#"
        body {
            background-color: #121212;
            color: #FFFFFF;
            font-family: Arial, sans-serif;
        }
        .centered-form {
            display: flex;
            justify-content: center;
            margin-bottom: 20px;
        }
        form {
            display: flex;
            flex-direction: column;
            width: 300px;
        }
        input[type="text"] {
            width: 50%;
        }
        textarea {
            height: 150px;
        }
        .post {
            border-bottom: 5px solid #333333;
            padding: 10px 0;
            position: relative;
        }
        .reply-button {
            position: absolute;
            top: 10px;
            right: 10px;
            background-color: #007bff;
            color: #ffffff;
            padding: 5px 10px;
            border-radius: 5px;
            text-decoration: none;
            cursor: pointer;
        }
        .reply-button:hover {
            background-color: #0056b3;
        }
        img, video {
            max-width: 200px;
            max-height: 200px;
            display: block;
            margin-bottom: 10px;
        }
        .pagination {
            text-align: center;
            margin-top: 20px;
        }
        .pagination a {
            color: #FFFFFF;
            padding: 5px 10px;
            text-decoration: none;
            border: 1px solid #FFFFFF;
            margin: 0 5px;
        }
        .pagination a:hover {
            background-color: #444444;
        }
        button {
            background-color: #007bff;
            color: #ffffff;
            padding: 10px;
            border: none;
            border-radius: 5px;
            cursor: pointer;
        }
        button:hover {
            background-color: #0056b3;
        }
    "#);
    body.push_str("</style></head><body>");
    body.push_str(r#"<div class="centered-form">"#);
    body.push_str(
        r#"<form action="/upload" method="post" enctype="multipart/form-data">
            <input type="hidden" name="parent_id" value="0">
            <input type="text" name="title" maxlength="20" placeholder="Title" required><br>
            <textarea name="message" maxlength="40000" placeholder="Message" required></textarea><br>
            <input type="file" name="file"><br>
            <button type="submit">Upload</button>
        </form>"#,
    );
    body.push_str(r#"</div>"#);

    for post in posts {
        let (id, post_id, title, message, file_path) = post.unwrap();

        let reply_count: i32 = conn.query_row(
            "SELECT COUNT(*) FROM files WHERE parent_id = ?1",
            params![id],
            |row| row.get(0),
        ).unwrap_or(0);

        body.push_str("<div class=\"post\">");
        body.push_str(&format!("<div class=\"post-id\">{}</div>", post_id));
        body.push_str(&format!("<div class=\"post-title\">{}</div>", title));
        if let Some(file_path) = file_path {
            if file_path.ends_with(".jpg") || file_path.ends_with(".jpeg") || file_path.ends_with(".png") || file_path.ends_with(".gif") || file_path.ends_with(".webp") {
                body.push_str(&format!(r#"<img src="/static/{}"><br>"#, file_path.trim_start_matches("./static/")));
            } else if file_path.ends_with(".mp4") || file_path.ends_with(".mp3") || file_path.ends_with(".webm") {
                body.push_str(&format!(r#"<video controls><source src="/static/{}"></video><br>"#, file_path.trim_start_matches("./static/")));
            }
        }
        body.push_str(&format!("<div class=\"post-message\">{}</div>", message));
        body.push_str(&format!("<a class=\"reply-button\" href=\"/post/{}\">Reply ({})</a>", id, reply_count));
        body.push_str("</div>");
    }

    let next_page = page + 1;
    let prev_page = if page > 1 { page - 1 } else { 1 };
    body.push_str("<div class=\"pagination\">");
    if page > 1 {
        body.push_str(&format!(r#"<a href="/?page={}">Previous</a>"#, prev_page));
    }
    body.push_str(&format!(r#"<a href="/?page={}">Next</a>"#, next_page));
    body.push_str("</div>");

    body.push_str("</body></html>");

    Ok(HttpResponse::Ok().content_type("text/html").body(body))
}

fn initialize_db() -> SqlResult<Connection> {
    let conn = Connection::open("my_database.db")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS files (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            post_id TEXT NOT NULL,
            parent_id INTEGER,
            title TEXT NOT NULL,
            message TEXT NOT NULL,
            file_path TEXT
        )",
        [],
    )?;
    Ok(conn)
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let conn = initialize_db().unwrap();
    let conn_data = Data::new(Mutex::new(conn));

    HttpServer::new(move || {
        App::new()
            .app_data(conn_data.clone())
            .app_data(Data::new(web::JsonConfig::default().limit(MAX_SIZE)))
            .service(
                web::resource("/")
                    .route(web::get().to(index))
            )
            .service(
                web::resource("/upload")
                    .route(web::post().to(save_file))
            )
            .service(
                web::resource("/post/{id}")
                    .route(web::get().to(view_post))
            )
            .service(fs::Files::new("/static", "./static").show_files_listing())
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}
