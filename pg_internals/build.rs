#[path = "build/configure.rs"]
mod configure;
#[path = "build/constants.rs"]
mod constants;
#[path = "build/guc.rs"]
mod guc;

fn main() {
    configure::build();
    constants::build();
    guc::build();
}
