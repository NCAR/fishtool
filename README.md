This tool implements a small number of operations against the DMTF Redfish API. It is intended for large cluster systems, currently it has been tested primarily against the HPE/Cray Redfish implementation. Currently this is a mostly internal debugging tool but it may someday turn into a more fully featured replacement for tools like ipmitool or IBM's r* family of tools. Currently proof-of-concept quality code.

## Building 

```
cargo build --release
```

If you don't have Rust installed and you're feeling very trusting, the easiest way is:

```
module purge --force
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup toolchain install stable
rustup run stable cargo build --release
```

## Run
* Single node:
```
target/release/fishtool -u root -p APassWord -n x9000c0s0b0 healthcheck
```

* Clush noderange (if clush is in your path)
```
target/release/fishtool -u root -p APassWord -N @x9000c[1-2]s0b0 healthcheck
target/release/fishtool -u root -p APassWord -N @bmc healthcheck
```

* General usage
```
target/release/fishtool --help
```

