use num_traits::ToPrimitive;
use std::env;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=assets/icon.svg");

    if let Some(commit) = git_commit_hash() {
        println!("cargo:rustc-env=RAIRSTREAM_GIT_COMMIT={commit}");
    }

    if target_os() == "windows" {
        embed_windows_icon().unwrap_or_else(|error| {
            panic!("failed to embed Windows icon: {error}");
        });
    }
}

fn git_commit_hash() -> Option<String> {
    let git_dir = Path::new(".git");
    let head_path = git_dir.join("HEAD");
    let head = fs::read_to_string(&head_path).ok()?;
    let head = head.trim();

    if let Some(reference) = head.strip_prefix("ref: ") {
        let reference_path = git_dir.join(reference);
        println!("cargo:rerun-if-changed={}", reference_path.display());
        read_ref(&reference_path)
    } else if head.is_empty() {
        None
    } else {
        Some(short_hash(head))
    }
}

fn read_ref(path: &Path) -> Option<String> {
    if let Ok(commit) = fs::read_to_string(path) {
        let commit = commit.trim();
        if !commit.is_empty() {
            return Some(short_hash(commit));
        }
    }

    let packed_refs_path = PathBuf::from(".git").join("packed-refs");
    println!("cargo:rerun-if-changed={}", packed_refs_path.display());
    let packed_refs = fs::read_to_string(packed_refs_path).ok()?;
    let target = path.strip_prefix(".git").ok()?.to_string_lossy();
    for line in packed_refs.lines() {
        if line.is_empty() || line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let commit = parts.next()?;
        let reference = parts.next()?;
        if reference == target {
            return Some(short_hash(commit));
        }
    }
    None
}

fn short_hash(commit: &str) -> String {
    commit.chars().take(7).collect()
}

fn target_os() -> String {
    env::var("CARGO_CFG_TARGET_OS").unwrap_or_default()
}

fn embed_windows_icon() -> Result<(), String> {
    let svg_path = Path::new("assets/icon.svg");
    let ico_path = render_ico(svg_path)?;
    let icon_path = ico_path.to_string_lossy().into_owned();

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(&icon_path);
    resource.compile().map_err(|error| error.to_string())
}

fn render_ico(svg_path: &Path) -> Result<PathBuf, String> {
    let options = resvg::usvg::Options {
        resources_dir: svg_path.parent().map(Path::to_path_buf),
        ..resvg::usvg::Options::default()
    };
    let svg_data = fs::read(svg_path)
        .map_err(|error| format!("failed to read {}: {error}", svg_path.display()))?;
    let tree = resvg::usvg::Tree::from_data(&svg_data, &options)
        .map_err(|error| format!("failed to parse {}: {error}", svg_path.display()))?;

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?);
    let ico_path = out_dir.join("rairstream.ico");
    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);

    for size in [16, 24, 32, 48, 64, 128, 256] {
        icon_dir.add_entry(render_icon_entry(&tree, size)?);
    }

    let output = File::create(&ico_path)
        .map_err(|error| format!("failed to create {}: {error}", ico_path.display()))?;
    icon_dir
        .write(output)
        .map_err(|error| format!("failed to write {}: {error}", ico_path.display()))?;

    Ok(ico_path)
}

fn render_icon_entry(tree: &resvg::usvg::Tree, size: u32) -> Result<ico::IconDirEntry, String> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)
        .ok_or_else(|| format!("failed to allocate {size}x{size} pixmap"))?;

    let icon_size = tree.size();
    let scale_x = f64::from(size) / f64::from(icon_size.width());
    let scale_y = f64::from(size) / f64::from(icon_size.height());
    let scale_x = scale_x
        .to_f32()
        .ok_or_else(|| format!("failed to convert horizontal scale {scale_x} to f32"))?;
    let scale_y = scale_y
        .to_f32()
        .ok_or_else(|| format!("failed to convert vertical scale {scale_y} to f32"))?;
    resvg::render(
        tree,
        resvg::tiny_skia::Transform::from_scale(scale_x, scale_y),
        &mut pixmap.as_mut(),
    );

    let mut rgba_data = Vec::with_capacity((size * size * 4) as usize);
    for pixel in pixmap.pixels() {
        // ICO expects straight-alpha RGBA, while tiny-skia stores premultiplied pixels.
        let color = pixel.demultiply();
        rgba_data.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }

    let image = ico::IconImage::from_rgba_data(size, size, rgba_data);
    ico::IconDirEntry::encode(&image)
        .map_err(|error| format!("failed to encode {size}x{size} icon entry: {error}"))
}
