#[path = "build/configure.rs"]
mod configure;
#[path = "build/constants.rs"]
mod constants;
#[path = "build/guc.rs"]
mod guc;
#[path = "build/guc_options.rs"]
mod guc_options;
#[path = "build/guc_parse.rs"]
mod guc_parse;
#[path = "build/guc_sources.rs"]
mod guc_sources;
#[path = "build/source.rs"]
mod source;

fn main() {
    let source = source::prepare();
    configure::build(&source);
    constants::build(&source);
    guc::build(&source);
    guc_parse::build(&source);
}
