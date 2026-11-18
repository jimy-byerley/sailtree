Name:       sailtree
Summary:    Local and remote file explorer
Version:    0.1.0
Release:    1
License:    LicenseRef-TBD
URL:        https://github.com/jimy-byerley/sailtree
Source0:    %{name}-%{version}.tar.bz2
Requires:   sailfishsilica-qt5 >= 0.10.9
Requires:   libsailfishapp
BuildRequires:  rust >= 1.75
BuildRequires:  cargo >= 1.75
BuildRequires:  rust-std-static
BuildRequires:  pkgconfig(sailfishapp) >= 1.0.2
BuildRequires:  pkgconfig(Qt5Core)
BuildRequires:  pkgconfig(Qt5Gui)
BuildRequires:  pkgconfig(Qt5Qml)
BuildRequires:  pkgconfig(Qt5Quick)
BuildRequires:  qt5-qtdeclarative-import-qttest
BuildRequires:  desktop-file-utils

%description
File explorer for the local device and remote devices (sftp, ftp).

# the binary is built by cargo, rpm debuginfo extraction is not set up for it
%define debug_package %{nil}

%prep
%setup -q -n %{name}-%{version}

# environment shared by the build and install steps
%define cargo_env \
    case "%{_arch}" in \
        arm*) RUST_TRIPLE=armv7-unknown-linux-gnueabihf ;; \
        aarch64) RUST_TRIPLE=aarch64-unknown-linux-gnu ;; \
        *) RUST_TRIPLE=i686-unknown-linux-gnu ;; \
    esac; \
    export SB2_RUST_TARGET_TRIPLE=$RUST_TRIPLE CARGO_BUILD_TARGET=$RUST_TRIPLE; \
    export CARGO_HOME=$PWD/target/cargo-home; \
    export CARGO_TARGET_DIR=$PWD/target; \
    export QMAKE=%{_bindir}/qmake; \
    export SB2_RUST_EXECVP_SHIM="/usr/bin/env LD_PRELOAD=/usr/lib/libsb2/libsb2.so.1 /usr/bin/env"; \
    export SB2_RUST_USE_REAL_EXECVP=Yes SB2_RUST_USE_REAL_FN=Yes SB2_RUST_NO_SPAWNVP=Yes; \
    case "$(stat -f -c %%T .)" in \
        fuse*|vboxsf) export CARGO_TARGET_DIR=$HOME/cargo-target/%{name} ;; \
    esac

%build
# QMAKE is explicit because probing for a missing qmake6 fails with a bogus error under scratchbox.
# rustc needs mmap for its build products, which the shared folders of the SDK's VirtualBox
# build engine (fuse) do not support: there the target dir goes to the engine's own disk.
# the SB2_RUST_* settings and the single job avoid cargo hanging between fork and exec under scratchbox.
%cargo_env
cargo build --release --locked -j1

%install
rm -rf %{buildroot}
%cargo_env
install -D -m 755 $CARGO_TARGET_DIR/$RUST_TRIPLE/release/%{name} %{buildroot}%{_bindir}/%{name}
mkdir -p %{buildroot}%{_datadir}/%{name}
cp -r qml %{buildroot}%{_datadir}/%{name}/qml
install -D -m 644 %{name}.desktop %{buildroot}%{_datadir}/applications/%{name}.desktop
for size in 86x86 108x108 128x128 172x172; do
    install -D -m 644 icons/$size/%{name}.png %{buildroot}%{_datadir}/icons/hicolor/$size/apps/%{name}.png
done
desktop-file-install --delete-original --dir %{buildroot}%{_datadir}/applications %{buildroot}%{_datadir}/applications/*.desktop

%files
%defattr(-,root,root,-)
%{_bindir}/%{name}
%{_datadir}/%{name}
%{_datadir}/applications/%{name}.desktop
%{_datadir}/icons/hicolor/*/apps/%{name}.png
