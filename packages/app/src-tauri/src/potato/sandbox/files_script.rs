//! The file browser's guest script (`packages/potato-sandbox/guest/files/pm-sbx-files.sh`), compiled into the app and run
//! as the sandbox user through `Sandbox::files_run`. Every path check lives in that script, in the computer, where the
//! symbolic links are real; the host side (`potato/files_browser.rs`) only builds the arguments and reads the answer.

/// The script text, with any carriage returns a Windows checkout may have added taken out (the guest's sh would choke on them).
pub fn script() -> String {
    include_str!("../../../../../potato-sandbox/guest/files/pm-sbx-files.sh").replace('\r', "")
}

/// What one run of the script printed: its exit code (`None`: stopped for taking too long) and its raw stdout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesOutput {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
}
