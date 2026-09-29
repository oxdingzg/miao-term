//! A minimal native terminal window using the self-drawn render loop.

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    miao_term_widget::run("miaotty-native")
}
