#[path = "build/configure.rs"]
mod configure;
#[path = "build/constants.rs"]
mod constants;
#[path = "build/guc.rs"]
mod guc;
#[path = "build/guc_options.rs"]
mod guc_options;

fn main() {
    configure::build();
    constants::build();
    guc::build();
}
