use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let product_config = manifest_dir.join("../config/product.properties");
    println!("cargo:rerun-if-changed={}", product_config.display());

    let content = fs::read_to_string(&product_config).expect("could not read product.properties");
    let properties: HashMap<_, _> = content
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect();

    export(
        &properties,
        "androidApplicationId",
        "GNIREHTET_NG_ANDROID_APPLICATION_ID",
    );
    export(
        &properties,
        "androidActivityClass",
        "GNIREHTET_NG_ANDROID_ACTIVITY_CLASS",
    );
    export(
        &properties,
        "androidVersionCode",
        "GNIREHTET_NG_ANDROID_VERSION_CODE",
    );
}

fn export(properties: &HashMap<&str, &str>, key: &str, environment_name: &str) {
    let value = properties
        .get(key)
        .unwrap_or_else(|| panic!("missing product property: {}", key));
    println!("cargo:rustc-env={}={}", environment_name, value);
}
