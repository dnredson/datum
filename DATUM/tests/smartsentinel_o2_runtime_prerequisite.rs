use datum::agent::runtime_prerequisite::{installation_args, PackageManager};

#[test]
fn apt_runtime_installation_uses_docker_io_without_shell_text() {
    assert_eq!(PackageManager::AptGet.executable(), "apt-get");
    assert_eq!(PackageManager::AptGet.docker_package(), "docker.io");
    assert_eq!(
        installation_args(PackageManager::AptGet),
        vec!["install", "-y", "docker.io"]
    );
}

#[test]
fn common_linux_package_managers_have_typed_docker_install_specs() {
    assert_eq!(PackageManager::Dnf.executable(), "dnf");
    assert_eq!(PackageManager::Yum.executable(), "yum");
    assert_eq!(PackageManager::Pacman.executable(), "pacman");
    assert_eq!(PackageManager::Zypper.executable(), "zypper");

    assert_eq!(
        installation_args(PackageManager::Pacman),
        vec!["-S", "--noconfirm", "docker"]
    );
    assert_eq!(PackageManager::Dnf.docker_package(), "moby-engine");
}
