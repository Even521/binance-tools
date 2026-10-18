use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=CUDA_PATH_V12_6");
    println!("cargo:rerun-if-env-changed=CUDA_PATH");
    if env::var_os("CARGO_FEATURE_CUDA").is_none() {
        return;
    }

    let cuda_root = env::var_os("CUDA_PATH_V12_6")
        .map(PathBuf::from)
        .or_else(|| env::var_os("CUDA_PATH").map(PathBuf::from))
        .expect("CUDA feature requires CUDA_PATH_V12_6 (CUDA Toolkit 12.x for Pascal sm_61)");
    let nvcc = cuda_root.join("bin").join("nvcc.exe");
    if !nvcc.exists() {
        panic!(
            "CUDA feature enabled, but nvcc was not found under {}",
            cuda_root.display()
        );
    }
    let output = Command::new(&nvcc)
        .arg("--version")
        .output()
        .expect("failed to run nvcc");
    println!("cargo:rerun-if-changed=cuda/bridge.cu");
    println!("cargo:rerun-if-changed=cuda/sha512_device.cuh");
    println!("cargo:rerun-if-changed=cuda/secp256k1_device.cuh");
    if !output.status.success() {
        panic!("failed to query CUDA compiler version");
    }
    let version_text = String::from_utf8_lossy(&output.stdout);
    if let Some(major) = cuda_major_version(&version_text)
        && major >= 13
    {
        panic!(
            "CUDA {major}.x cannot build Pascal sm_61; set CUDA_PATH_V12_6 to CUDA Toolkit 12.6"
        );
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set"));
    let archive = out_dir.join("evm_wallet_cuda.lib");
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir is missing"))
        .join("cuda/bridge.cu");
    let architecture = env::var("CUDA_ARCH").unwrap_or_else(|_| "sm_61".to_string());
    let gencode = gencode_for(&architecture);
    let mut script = String::new();
    if let Some(vcvars) = find_vcvars64() {
        script.push_str(&format!("call \"{}\" >nul\r\n", vcvars.display()));
        script.push_str("if errorlevel 1 exit /b 1\r\n");
    }
    script.push_str(&format!("set CUDA_PATH={}\r\n", cuda_root.display()));
    script.push_str(&format!(
        "\"{}\" --lib -Xcompiler=/MD -gencode={} -o \"{}\" \"{}\"\r\n",
        nvcc.display(),
        gencode,
        archive.display(),
        source.display()
    ));
    script.push_str("exit /b %errorlevel%\r\n");
    let script_path = out_dir.join("compile_cuda.cmd");
    fs::write(&script_path, script).expect("failed to write CUDA build script");
    let status = Command::new("cmd.exe")
        .args(["/C", script_path.to_string_lossy().as_ref()])
        .status()
        .expect("failed to launch CUDA compiler command");
    if !status.success() {
        panic!("nvcc failed to compile CUDA sources for {architecture}");
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!(
        "cargo:rustc-link-search=native={}",
        cuda_root.join("lib/x64").display()
    );
    println!("cargo:rustc-link-lib=static=evm_wallet_cuda");
    println!("cargo:rustc-link-lib=dylib=cudart");
    println!("cargo:rustc-link-lib=dylib=user32");
    println!("cargo:rustc-link-lib=dylib=advapi32");

    let runtime_dll = cuda_root.join("bin/cudart64_12.dll");
    let profile_dir = out_dir.ancestors().nth(3).expect("invalid OUT_DIR path");
    let runtime_target = profile_dir.join("cudart64_12.dll");
    if runtime_target.exists() {
        let current = fs::read(&runtime_target).expect("failed to read existing CUDA runtime");
        let selected = fs::read(&runtime_dll).expect("failed to read CUDA runtime from toolkit");
        if current != selected {
            panic!(
                "a different cudart64_12.dll is already present beside a running build; close the app and rebuild"
            );
        }
    } else {
        fs::copy(runtime_dll, runtime_target)
            .expect("failed to copy CUDA runtime DLL beside the executable");
    }
}

fn cuda_major_version(version_text: &str) -> Option<u32> {
    let marker = "release ";
    let start = version_text.find(marker)? + marker.len();
    version_text[start..]
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

fn gencode_for(architecture: &str) -> String {
    let suffix = architecture
        .strip_prefix("sm_")
        .or_else(|| architecture.strip_prefix("compute_"))
        .unwrap_or("61");
    format!("arch=compute_{suffix},code=sm_{suffix}")
}

fn find_vcvars64() -> Option<PathBuf> {
    if let Some(path) = env::var_os("VCVARS64_BAT").map(PathBuf::from) {
        if path.exists() {
            return Some(path);
        }
    }
    let common = [
        PathBuf::from(r"E:\app\vs-buildtools-binance\VC\Auxiliary\Build\vcvars64.bat"),
        PathBuf::from(
            r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
        ),
        PathBuf::from(
            r"C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
        ),
    ];
    common.into_iter().find(|path| path.exists())
}
