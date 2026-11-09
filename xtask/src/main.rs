use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 || args[1] != "bundle" {
        eprintln!("Uso: cargo xtask bundle <paquete> [--release]");
        std::process::exit(1);
    }

    let pkg = &args[2];
    let release = args.contains(&"--release".to_string());
    
    // 1. Compilar el paquete
    let mut cmd = Command::new("cargo");
    cmd.arg("build").arg("-p").arg(pkg);
    if release {
        cmd.arg("--release");
    }
    
    println!("📦 Compilando {}...", pkg);
    let status = cmd.status().expect("Fallo al ejecutar cargo build");
    if !status.success() {
        eprintln!("❌ Error al compilar el paquete.");
        std::process::exit(1);
    }

    // 2. Determinar el directorio target y el archivo .so
    let target_dir = if release { "target/release" } else { "target/debug" };
    let so_name = format!("lib{}.so", pkg.replace("-", "_"));
    let so_path = PathBuf::from(target_dir).join(&so_name);
    
    if !so_path.exists() {
        eprintln!("❌ No se encontró el archivo {}", so_path.display());
        std::process::exit(1);
    }

    // 3. Crear el directorio bundled
    let bundle_dir = PathBuf::from("target/bundled");
    fs::create_dir_all(&bundle_dir).unwrap();

    // 4. Copiar a CLAP y renombrar
    let clap_name = "CAJA.Groove.Pattern.clap";
    let clap_path = bundle_dir.join(clap_name);
    fs::copy(&so_path, &clap_path).unwrap();
    println!("✅ CLAP generado: target/bundled/{}", clap_name);

    // 5. Copiar a VST3 (estructura correcta)
    let vst3_name = "CAJA.Groove.Pattern.vst3";
    let vst3_dir = bundle_dir.join(vst3_name);
    let contents_dir = vst3_dir.join("Contents").join("x86_64-linux");
    fs::create_dir_all(&contents_dir).unwrap();
    let vst3_so_path = contents_dir.join("CAJA.Groove.Pattern.so");
    fs::copy(&so_path, &vst3_so_path).unwrap();
    println!("✅ VST3 generado: target/bundled/{}", vst3_name);
    
    println!("\n🎉 ¡Listo! Ahora puedes copiar los plugins a tus carpetas:");
    println!("   mkdir -p ~/.clap ~/.vst3");
    println!("   cp target/bundled/CAJA.Groove.Pattern.clap ~/.clap/");
    println!("   cp -r target/bundled/CAJA.Groove.Pattern.vst3 ~/.vst3/");
}
