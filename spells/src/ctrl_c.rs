use tokio_util::sync::CancellationToken;

pub fn install_ctrlc_handler() -> CancellationToken {
    let ctrlc = CancellationToken::new();
    ctrlc::set_handler({
        let mut already_called = false;
        let ctrlc = ctrlc.clone();
        move || {
            if already_called {
                println!("Ctrl-C pressed again, exiting immediately");
                std::process::exit(0);
            } else {
                already_called = true;
            }
            println!("Ctrl-C pressed, cancelling runtime");
            ctrlc.cancel();
        }
    })
    .expect("Failed to set Ctrl-C handler");
    ctrlc
}

pub fn install_ctrlc_handler_f<F: Fn() + Send + 'static>(f: F) -> CancellationToken {
    let ctrlc = CancellationToken::new();
    ctrlc::set_handler({
        let mut already_called = false;
        let ctrlc = ctrlc.clone();
        move || {
            if already_called {
                println!("Ctrl-C pressed again, exiting immediately");
                std::process::exit(0);
            } else {
                already_called = true;
            }
            println!("Ctrl-C pressed, cancelling runtime");
            f();
            ctrlc.cancel();
        }
    })
    .expect("Failed to set Ctrl-C handler");
    ctrlc
}
