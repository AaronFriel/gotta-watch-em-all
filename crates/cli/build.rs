fn main() {
    embed_resource::compile(
        "gotta-watch-em-all-manifest.rc",
        embed_resource::NONE,
    )
    .manifest_optional()
    .unwrap();
}
