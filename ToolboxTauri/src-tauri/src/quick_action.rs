use crate::kit::common::batch_runner::BatchRunner;
use crate::kit::common::JobOutcome;
use crate::kit::contracts::OutputLocation;
use crate::kit::images::{ImageProcessor, Options as ImageOptions, OutputFormat};
use crate::kit::pdf::remaining;
use crate::kit::vision;
use std::path::{Path, PathBuf};

pub const FLAG: &str = "--quick-action";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind { Pdf, Image }

impl FileKind {
    fn extensions(self) -> &'static [&'static str] {
        match self {
            FileKind::Pdf => &["pdf"],
            FileKind::Image => &["png", "jpg", "jpeg", "webp", "heic", "heif", "gif", "tif", "tiff"],
        }
    }

    fn uti(self) -> &'static str {
        match self {
            FileKind::Pdf => "com.adobe.pdf",
            FileKind::Image => "public.image",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickAction { CompressPdf, PdfToImages, CompressImage, ConvertToPng, ConvertToJpg, RemoveBackground }

impl QuickAction {
    pub const ALL: [QuickAction; 6] = [
        QuickAction::CompressPdf,
        QuickAction::PdfToImages,
        QuickAction::CompressImage,
        QuickAction::ConvertToPng,
        QuickAction::ConvertToJpg,
        QuickAction::RemoveBackground,
    ];

    pub fn id(self) -> &'static str {
        match self {
            QuickAction::CompressPdf => "compress-pdf",
            QuickAction::PdfToImages => "pdf-to-images",
            QuickAction::CompressImage => "compress-image",
            QuickAction::ConvertToPng => "convert-to-png",
            QuickAction::ConvertToJpg => "convert-to-jpg",
            QuickAction::RemoveBackground => "remove-background",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            QuickAction::CompressPdf => "Compress PDF",
            QuickAction::PdfToImages => "Convert PDF to Images",
            QuickAction::CompressImage => "Compress Image",
            QuickAction::ConvertToPng => "Convert Image to PNG",
            QuickAction::ConvertToJpg => "Convert Image to JPG",
            QuickAction::RemoveBackground => "Remove Image Background",
        }
    }

    pub fn tool(self) -> &'static str {
        match self {
            QuickAction::CompressPdf => "compress_pdf",
            QuickAction::PdfToImages => "pdf_to_images",
            QuickAction::CompressImage => "compress_images",
            QuickAction::ConvertToPng | QuickAction::ConvertToJpg => "convert_images",
            QuickAction::RemoveBackground => "remove_image_background",
        }
    }

    pub fn kind(self) -> FileKind {
        match self {
            QuickAction::CompressPdf | QuickAction::PdfToImages => FileKind::Pdf,
            _ => FileKind::Image,
        }
    }

    pub fn parse(id: &str) -> Option<QuickAction> {
        QuickAction::ALL.into_iter().find(|action| action.id() == id)
    }

    pub fn run(self, paths: Vec<PathBuf>) -> Vec<JobOutcome> {
        let output_location = OutputLocation::AlongsideInput;
        match self {
            QuickAction::CompressPdf => {
                let request = remaining::CompressPdfRequest { paths: paths.clone(), quality: 80, output_location };
                BatchRunner::run(paths, |path| remaining::compress(&request, path))
            }
            QuickAction::PdfToImages => {
                let request = remaining::PdfToImagesRequest { paths: paths.clone(), dpi: 150, format: "png".to_string(), page_range: None, output_location };
                BatchRunner::run(paths, |path| remaining::to_images(&request, path))
            }
            QuickAction::CompressImage => BatchRunner::run(paths, |path| ImageProcessor::run(path, ImageOptions {
                target_format: None,
                quality: 80,
                keep_smaller_original: true,
                suffix: "-compressed".to_string(),
                output_location: output_location.clone(),
            })),
            QuickAction::ConvertToPng | QuickAction::ConvertToJpg => {
                let format = if self == QuickAction::ConvertToPng { OutputFormat::Png } else { OutputFormat::Jpeg };
                BatchRunner::run(paths, |path| ImageProcessor::run(path, ImageOptions {
                    target_format: Some(format),
                    quality: 80,
                    keep_smaller_original: false,
                    suffix: "-converted".to_string(),
                    output_location: output_location.clone(),
                }))
            }
            QuickAction::RemoveBackground => {
                let request = vision::VisionRequest { paths: paths.clone(), output_location };
                BatchRunner::run(paths, |path| vision::remove_background(&request, path))
            }
        }
    }
}

pub struct Invocation { pub action: QuickAction, pub paths: Vec<PathBuf> }

/// `None` means the process was not launched as a quick action and should start the app.
pub fn parse_args(args: &[String]) -> Option<Result<Invocation, String>> {
    let position = args.iter().position(|arg| arg == FLAG)?;
    let Some(id) = args.get(position + 1) else { return Some(Err("Missing quick action name.".to_string())) };
    let Some(action) = QuickAction::parse(id) else { return Some(Err(format!("Unknown quick action: {id}"))) };
    let paths: Vec<PathBuf> = args[position + 2..].iter().map(PathBuf::from).collect();
    if paths.is_empty() { return Some(Err("No files were selected.".to_string())); }
    Some(Ok(Invocation { action, paths }))
}

pub fn summary(action: QuickAction, outcomes: &[JobOutcome]) -> String {
    let failures: Vec<&JobOutcome> = outcomes.iter().filter(|outcome| outcome.failure.is_some()).collect();
    let done = outcomes.len() - failures.len();
    match failures.first().and_then(|outcome| outcome.failure.as_ref()) {
        None => format!("{}: {done} file(s) done.", action.title()),
        Some(error) => format!("{}: {done} done, {} failed. {error}", action.title(), failures.len()),
    }
}

pub fn notify(title: &str, body: &str) {
    let _ = notify_rust::Notification::new().summary(title).body(body).show();
}

/// The actions the user chose to show in the file manager. A missing or unreadable file means all of them.
pub fn load_enabled(path: &Path) -> Vec<QuickAction> {
    std::fs::read_to_string(path).ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .map(|ids| QuickAction::ALL.into_iter().filter(|action| ids.iter().any(|id| id == action.id())).collect())
        .unwrap_or_else(|| QuickAction::ALL.to_vec())
}

pub fn save_enabled(path: &Path, enabled: &[QuickAction]) -> Result<(), String> {
    let ids: Vec<&str> = enabled.iter().map(|action| action.id()).collect();
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
    std::fs::write(path, serde_json::to_string(&ids).unwrap()).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

/// Registers the chosen context-menu entries for the running executable and removes the rest.
/// Rewrites only what changed, so it is safe to call on every launch and follows the app when it moves.
pub fn register(executable: &Path, enabled: &[QuickAction]) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return macos::install(executable, &macos::services_dir()?, enabled).and_then(|_| macos::enable(enabled));
    #[cfg(target_os = "windows")]
    return windows::install(executable, enabled);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    { let _ = (executable, enabled); Ok(()) }
}

pub mod macos {
    use super::QuickAction;
    use std::path::{Path, PathBuf};

    pub fn services_dir() -> Result<PathBuf, String> {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Services")).ok_or_else(|| "HOME is not set.".to_string())
    }

    pub fn workflow_name(action: QuickAction) -> String { format!("Toolbox - {}.workflow", action.title()) }

    pub fn install(executable: &Path, services: &Path, enabled: &[QuickAction]) -> Result<(), String> {
        let mut changed = false;
        for action in QuickAction::ALL {
            let bundle = services.join(workflow_name(action));
            if !enabled.contains(&action) {
                if bundle.exists() {
                    std::fs::remove_dir_all(&bundle).map_err(|e| format!("Cannot remove {}: {e}", bundle.display()))?;
                    changed = true;
                }
                continue;
            }
            let contents = (info_plist(action), document(action, executable));
            let bundle = services.join(workflow_name(action)).join("Contents");
            changed |= write_if_changed(&bundle.join("Info.plist"), &contents.0)?;
            changed |= write_if_changed(&bundle.join("document.wflow"), &contents.1)?;
        }
        if changed {
            let _ = std::process::Command::new("/System/Library/CoreServices/pbs").arg("-update").status();
        }
        Ok(())
    }

    const VISIBLE: &str = "<dict><key>presentation_modes</key><dict><key>ContextMenu</key><true/><key>FinderPreview</key><true/><key>ServicesMenu</key><true/><key>TouchBar</key><true/></dict></dict>";

    pub fn status_key(action: QuickAction) -> String {
        format!("(null) - Toolbox: {} - runWorkflowAsService", action.title())
    }

    /// Finder lists a workflow only once the services status marks it visible; Automator does this on save.
    pub fn enable(enabled: &[QuickAction]) -> Result<(), String> {
        for &action in enabled {
            let status = std::process::Command::new("defaults")
                .args(["write", "pbs", "NSServicesStatus", "-dict-add", &format!("\"{}\"", status_key(action)), VISIBLE])
                .status()
                .map_err(|e| format!("Cannot enable {}: {e}", action.title()))?;
            if !status.success() { return Err(format!("Cannot enable {}.", action.title())); }
        }
        let _ = std::process::Command::new("/System/Library/CoreServices/pbs").arg("-update").status();
        Ok(())
    }

    fn write_if_changed(path: &Path, contents: &str) -> Result<bool, String> {
        if std::fs::read_to_string(path).ok().as_deref() == Some(contents) { return Ok(false); }
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
        std::fs::write(path, contents).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
        Ok(true)
    }

    fn xml(text: &str) -> String {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }

    fn shell_quote(text: &str) -> String { format!("'{}'", text.replace('\'', "'\\''")) }

    pub fn command(action: QuickAction, executable: &Path) -> String {
        format!("exec {} {} {} \"$@\"", shell_quote(&executable.to_string_lossy()), super::FLAG, action.id())
    }

    pub fn info_plist(action: QuickAction) -> String {
        format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>NSServices</key>
	<array>
		<dict>
			<key>NSBackgroundColorName</key>
			<string>background</string>
			<key>NSIconName</key>
			<string>NSActionTemplate</string>
			<key>NSMenuItem</key>
			<dict>
				<key>default</key>
				<string>Toolbox: {title}</string>
			</dict>
			<key>NSMessage</key>
			<string>runWorkflowAsService</string>
			<key>NSRequiredContext</key>
			<dict>
				<key>NSApplicationIdentifier</key>
				<string>com.apple.finder</string>
			</dict>
			<key>NSSendFileTypes</key>
			<array>
				<string>{uti}</string>
			</array>
		</dict>
	</array>
</dict>
</plist>
"#, title = xml(action.title()), uti = action.kind().uti())
    }

    pub fn document(action: QuickAction, executable: &Path) -> String {
        format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AMApplicationBuild</key>
	<string>523</string>
	<key>AMApplicationVersion</key>
	<string>2.10</string>
	<key>AMDocumentVersion</key>
	<string>2</string>
	<key>actions</key>
	<array>
		<dict>
			<key>action</key>
			<dict>
				<key>AMAccepts</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Optional</key>
					<true/>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>AMActionVersion</key>
				<string>2.0.3</string>
				<key>AMApplication</key>
				<array>
					<string>Automator</string>
				</array>
				<key>AMParameterProperties</key>
				<dict>
					<key>COMMAND_STRING</key>
					<dict/>
					<key>CheckedForUserDefaultShell</key>
					<dict/>
					<key>inputMethod</key>
					<dict/>
					<key>shell</key>
					<dict/>
					<key>source</key>
					<dict/>
				</dict>
				<key>AMProvides</key>
				<dict>
					<key>Container</key>
					<string>List</string>
					<key>Types</key>
					<array>
						<string>com.apple.cocoa.string</string>
					</array>
				</dict>
				<key>ActionBundlePath</key>
				<string>/System/Library/Automator/Run Shell Script.action</string>
				<key>ActionName</key>
				<string>Run Shell Script</string>
				<key>ActionParameters</key>
				<dict>
					<key>COMMAND_STRING</key>
					<string>{command}</string>
					<key>CheckedForUserDefaultShell</key>
					<true/>
					<key>inputMethod</key>
					<integer>1</integer>
					<key>shell</key>
					<string>/bin/bash</string>
					<key>source</key>
					<string></string>
				</dict>
				<key>BundleIdentifier</key>
				<string>com.apple.RunShellScript</string>
				<key>CFBundleVersion</key>
				<string>2.0.3</string>
				<key>CanShowSelectedItemsWhenRun</key>
				<false/>
				<key>CanShowWhenRun</key>
				<true/>
				<key>Category</key>
				<array>
					<string>AMCategoryUtilities</string>
				</array>
				<key>Class Name</key>
				<string>RunShellScriptAction</string>
				<key>InputUUID</key>
				<string>5E4B9C1A-7C55-4F0E-9E21-1B7A0C7E0001</string>
				<key>OutputUUID</key>
				<string>5E4B9C1A-7C55-4F0E-9E21-1B7A0C7E0002</string>
				<key>UUID</key>
				<string>5E4B9C1A-7C55-4F0E-9E21-1B7A0C7E0003</string>
				<key>UnlocalizedApplications</key>
				<array>
					<string>Automator</string>
				</array>
				<key>arguments</key>
				<dict/>
				<key>isViewVisible</key>
				<integer>1</integer>
				<key>location</key>
				<string>309.000000:253.000000</string>
			</dict>
			<key>isViewVisible</key>
			<integer>1</integer>
		</dict>
	</array>
	<key>connectors</key>
	<dict/>
	<key>workflowMetaData</key>
	<dict>
		<key>applicationBundleID</key>
		<string>com.apple.finder</string>
		<key>applicationBundleIDsByPath</key>
		<dict>
			<key>/System/Library/CoreServices/Finder.app</key>
			<string>com.apple.finder</string>
		</dict>
		<key>applicationPath</key>
		<string>/System/Library/CoreServices/Finder.app</string>
		<key>applicationPaths</key>
		<array>
			<string>/System/Library/CoreServices/Finder.app</string>
		</array>
		<key>inputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>outputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>presentationMode</key>
		<integer>15</integer>
		<key>processesInput</key>
		<false/>
		<key>serviceApplicationBundleID</key>
		<string>com.apple.finder</string>
		<key>serviceApplicationPath</key>
		<string>/System/Library/CoreServices/Finder.app</string>
		<key>serviceInputTypeIdentifier</key>
		<string>com.apple.Automator.fileSystemObject</string>
		<key>serviceOutputTypeIdentifier</key>
		<string>com.apple.Automator.nothing</string>
		<key>serviceProcessesInput</key>
		<false/>
		<key>systemImageName</key>
		<string>NSActionTemplate</string>
		<key>useAutomaticInputType</key>
		<false/>
		<key>workflowTypeIdentifier</key>
		<string>com.apple.Automator.servicesMenu</string>
	</dict>
</dict>
</plist>
"#, command = xml(&command(action, executable)))
    }
}

pub mod windows {
    use super::{FileKind, QuickAction};
    use std::path::Path;

    pub const MENU_KEY: &str = "Toolbox";

    /// Every extension that gets a Toolbox submenu. The NSIS uninstall hook deletes the same list.
    pub fn extensions() -> Vec<&'static str> {
        [FileKind::Pdf, FileKind::Image].into_iter().flat_map(|kind| kind.extensions().iter().copied()).collect()
    }

    pub fn menu_root(extension: &str) -> String {
        format!(r"HKCU\Software\Classes\SystemFileAssociations\.{extension}\shell\{MENU_KEY}")
    }

    /// (key, value name, data). An empty value name is the key's default value.
    pub fn registry_values(executable: &Path, enabled: &[QuickAction]) -> Vec<(String, &'static str, String)> {
        let exe = executable.to_string_lossy();
        let mut values = Vec::new();
        for kind in [FileKind::Pdf, FileKind::Image] {
            if !enabled.iter().any(|action| action.kind() == kind) { continue; }
            for extension in kind.extensions() {
                let root = menu_root(extension);
                values.push((root.clone(), "MUIVerb", "Toolbox".to_string()));
                values.push((root.clone(), "SubCommands", String::new()));
                values.push((root.clone(), "Icon", format!("\"{exe}\",0")));
                for &action in enabled.iter().filter(|action| action.kind() == kind) {
                    let verb = format!(r"{root}\shell\{}", action.id());
                    values.push((verb.clone(), "MUIVerb", action.title().to_string()));
                    values.push((format!(r"{verb}\command"), "", format!("\"{exe}\" {} {} \"%1\"", super::FLAG, action.id())));
                }
            }
        }
        values
    }

    #[cfg(target_os = "windows")]
    pub fn install(executable: &Path, enabled: &[QuickAction]) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        for extension in extensions() {
            // Clears verbs the user turned off; fails harmlessly when the key does not exist yet.
            let _ = std::process::Command::new("reg").args(["delete", &menu_root(extension), "/f"]).creation_flags(CREATE_NO_WINDOW).status();
        }
        for (key, name, data) in registry_values(executable, enabled) {
            let mut command = std::process::Command::new("reg");
            command.arg("add").arg(&key);
            if name.is_empty() { command.arg("/ve"); } else { command.arg("/v").arg(name); }
            let status = command.arg("/d").arg(&data).arg("/f").creation_flags(CREATE_NO_WINDOW).status()
                .map_err(|e| format!("Cannot run reg.exe: {e}"))?;
            if !status.success() { return Err(format!("reg.exe failed for {key}")); }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> { list.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn plain_launch_is_not_a_quick_action() {
        assert!(parse_args(&args(&["toolbox"])).is_none());
    }

    #[test]
    fn parses_action_and_paths() {
        let invocation = parse_args(&args(&["toolbox", FLAG, "compress-pdf", "/a.pdf", "/b c.pdf"])).unwrap().unwrap();
        assert_eq!(invocation.action, QuickAction::CompressPdf);
        assert_eq!(invocation.paths, vec![PathBuf::from("/a.pdf"), PathBuf::from("/b c.pdf")]);
    }

    #[test]
    fn rejects_unknown_action_and_missing_files() {
        assert!(parse_args(&args(&["toolbox", FLAG, "explode"])).unwrap().is_err());
        assert!(parse_args(&args(&["toolbox", FLAG, "compress-pdf"])).unwrap().is_err());
        assert!(parse_args(&args(&["toolbox", FLAG])).unwrap().is_err());
    }

    #[test]
    fn every_action_round_trips_its_id() {
        for action in QuickAction::ALL { assert_eq!(QuickAction::parse(action.id()), Some(action)); }
    }

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("toolbox_qa_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn convert_writes_new_file_and_keeps_original() {
        let dir = sandbox("convert");
        let input = dir.join("photo.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([10, 20, 30, 255])).save(&input).unwrap();
        let before = std::fs::read(&input).unwrap();
        let outcomes = QuickAction::ConvertToJpg.run(vec![input.clone()]);
        assert!(outcomes[0].failure.is_none(), "{:?}", outcomes[0].failure);
        assert!(outcomes[0].output_paths[0].is_file());
        assert_eq!(outcomes[0].output_paths[0].parent(), Some(dir.as_path()));
        assert_eq!(std::fs::read(&input).unwrap(), before, "original untouched");
        assert!(summary(QuickAction::ConvertToJpg, &outcomes).contains("1 file(s) done"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_bad_file_does_not_stop_the_batch() {
        let dir = sandbox("isolation");
        let good = dir.join("ok.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255])).save(&good).unwrap();
        let outcomes = QuickAction::CompressImage.run(vec![dir.join("missing.png"), good]);
        assert_eq!(outcomes.iter().filter(|o| o.failure.is_none()).count(), 1);
        assert!(summary(QuickAction::CompressImage, &outcomes).contains("1 done, 1 failed"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn macos_workflows_quote_the_executable_and_filter_by_type() {
        let exe = Path::new("/Applications/It's Toolbox.app/Contents/MacOS/toolbox");
        assert_eq!(macos::command(QuickAction::CompressPdf, exe),
            r#"exec '/Applications/It'\''s Toolbox.app/Contents/MacOS/toolbox' --quick-action compress-pdf "$@""#);
        assert!(macos::info_plist(QuickAction::CompressPdf).contains("<string>com.adobe.pdf</string>"));
        assert!(macos::info_plist(QuickAction::RemoveBackground).contains("<string>public.image</string>"));
        assert!(macos::document(QuickAction::CompressPdf, exe).contains("--quick-action compress-pdf &quot;$@&quot;"));
    }

    #[test]
    fn macos_status_key_matches_the_menu_title() {
        assert_eq!(macos::status_key(QuickAction::CompressPdf), "(null) - Toolbox: Compress PDF - runWorkflowAsService");
        assert!(macos::info_plist(QuickAction::CompressPdf).contains("<string>Toolbox: Compress PDF</string>"));
    }

    #[test]
    fn macos_install_writes_one_bundle_per_action_and_is_idempotent() {
        let dir = sandbox("services");
        let exe = Path::new("/Applications/Toolbox.app/Contents/MacOS/toolbox");
        macos::install(exe, &dir, &QuickAction::ALL).unwrap();
        for action in QuickAction::ALL {
            let bundle = dir.join(macos::workflow_name(action)).join("Contents");
            assert!(bundle.join("Info.plist").is_file());
            assert!(std::fs::read_to_string(bundle.join("document.wflow")).unwrap().contains(action.id()));
        }
        let stamp = std::fs::metadata(dir.join(macos::workflow_name(QuickAction::CompressPdf)).join("Contents/Info.plist")).unwrap().modified().unwrap();
        macos::install(exe, &dir, &QuickAction::ALL).unwrap();
        let again = std::fs::metadata(dir.join(macos::workflow_name(QuickAction::CompressPdf)).join("Contents/Info.plist")).unwrap().modified().unwrap();
        assert_eq!(stamp, again, "unchanged files are not rewritten");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// End to end through Automator: `TOOLBOX_QA_EXE=<toolbox binary> cargo test -- --ignored automator`.
    #[test]
    #[ignore]
    #[cfg(target_os = "macos")]
    fn automator_runs_the_workflow_against_the_real_binary() {
        let exe = PathBuf::from(std::env::var("TOOLBOX_QA_EXE").expect("TOOLBOX_QA_EXE"));
        let dir = sandbox("automator");
        let input = dir.join("shot.jpg");
        image::RgbImage::from_pixel(8, 8, image::Rgb([200, 100, 50])).save(&input).unwrap();
        macos::install(&exe, &dir.join("Services"), &QuickAction::ALL).unwrap();
        let workflow = dir.join("Services").join(macos::workflow_name(QuickAction::ConvertToPng));
        let status = std::process::Command::new("automator").arg("-i").arg(&input).arg(&workflow).status().unwrap();
        assert!(status.success());
        assert!(dir.join("shot-converted.png").is_file(), "workflow produced the PNG next to the input");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_verbs_match_file_types() {
        let values = windows::registry_values(Path::new(r"C:\Program Files\Toolbox\toolbox.exe"), &QuickAction::ALL);
        let pdf_commands: Vec<&String> = values.iter().filter(|(key, _, _)| key.contains(r"\.pdf\") && key.ends_with(r"\command")).map(|(_, _, data)| data).collect();
        assert_eq!(pdf_commands, vec![
            r#""C:\Program Files\Toolbox\toolbox.exe" --quick-action compress-pdf "%1""#,
            r#""C:\Program Files\Toolbox\toolbox.exe" --quick-action pdf-to-images "%1""#,
        ]);
        assert!(values.iter().any(|(key, _, data)| key.contains(r"\.png\") && data.contains("remove-background")));
        assert!(!values.iter().any(|(key, _, data)| key.contains(r"\.pdf\") && data.contains("convert-to-png")));
    }

    #[test]
    fn enabled_selection_round_trips_and_defaults_to_all() {
        let dir = sandbox("selection");
        let path = dir.join("quick-actions.json");
        assert_eq!(load_enabled(&path), QuickAction::ALL.to_vec());
        save_enabled(&path, &[QuickAction::RemoveBackground, QuickAction::CompressPdf]).unwrap();
        assert_eq!(load_enabled(&path), vec![QuickAction::CompressPdf, QuickAction::RemoveBackground]);
        save_enabled(&path, &[]).unwrap();
        assert!(load_enabled(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn macos_install_removes_actions_the_user_turned_off() {
        let dir = sandbox("services-off");
        let exe = Path::new("/Applications/Toolbox.app/Contents/MacOS/toolbox");
        macos::install(exe, &dir, &QuickAction::ALL).unwrap();
        macos::install(exe, &dir, &[QuickAction::CompressImage]).unwrap();
        for action in QuickAction::ALL {
            assert_eq!(dir.join(macos::workflow_name(action)).exists(), action == QuickAction::CompressImage, "{}", action.id());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_skips_file_types_with_no_chosen_action() {
        let values = windows::registry_values(Path::new(r"C:\toolbox.exe"), &[QuickAction::CompressPdf]);
        assert!(values.iter().all(|(key, _, _)| key.contains(r"\.pdf\")));
        assert!(!values.iter().any(|(_, _, data)| data.contains("pdf-to-images")));
    }

    #[test]
    fn nsis_uninstall_hook_removes_every_registered_extension() {
        let hook = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/windows/hooks.nsh")).unwrap();
        for extension in windows::extensions() {
            let key = windows::menu_root(extension).replacen(r"HKCU\", "", 1);
            assert!(hook.contains(&format!("DeleteRegKey HKCU \"{key}\"")), "hook misses .{extension}");
        }
    }
}
