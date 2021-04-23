# Tendermint KMS Light 🔐

TEE-based Key Management System for [Tendermint](http://github.com/tendermint/tendermint/) validators.

## About

This repository contains `tmkms-light`, a key management service intended to be deployed
in conjunction with [Tendermint](http://github.com/tendermint/tendermint/) applications (ideally on separate physical hosts).
The code is based on the [tmkms](https://github.com/iqlusioninc/tmkms) repository with the following differences:

- **Smaller codebase** with the following limitations:
1. only one network can be configured in each `tmkms-light` process;
2. only the latest Tendermint protocol (v0.34) is supported;
3. there is no support for HSMs;
4. there is no support for transaction signing (`tx-signer` feature);
5. only `x86_64` Linux is supported.

- The code does not use conditional compilation (each signing provider has separate binaries) and dependencies on `abscissa` crates were replaced in order to fully support `tracing` for the application logging.
- **Signing in TEE (Trusted Execution Environments)**: currently, Intel(R) SGX and AWS Nitro Enclaves are supported.

## Status

Tendermint KMS Light is still in development (e.g. the SGX signing provider has not
yet been signed, so that it could be launched in the production mode).
In the future, the work developed in this repository may be upstreamed 
to the original [tmkms](https://github.com/iqlusioninc/tmkms) repository.

## Signing Providers

The following signing backend providers are presently supported:

### Software-Only (not recommended; only for testing)

This is contained in the "providers/softsign" directory.

### Intel(R) SGX
This is contained in the "providers/sgx" directory.
There are two crates that need to be compiled separately:
* `tmkms-light-sgx-app`: this is the enclave application that needs to be compiled for the `x86_64-fortanix-unknown-sgx` target, converted and signed as per [EDP instructions](https://edp.fortanix.com/docs/tasks/deployment/);
* `tmkms-light-sgx-runner`: this is the (host) runner application that is used to load the enclave application
and interface with the host system.

#### Installation

First, Setup your sgx machine with [EDP Installation guide](https://edp.fortanix.com/docs/installation/guide/)

Build `tmkms-light-sgx-runner`
```bash
cargo build --target x86_64-unknown-linux-gnu -p tmkms-light-sgx-runner --release
```
Build `tmkms-light-sgx-app`
```bash
cargo build --target x86_64-fortanix-unknown-sgx -p tmkms-light-sgx-app --release
```

Follow [EDP instructions](https://edp.fortanix.com/docs/tasks/deployment/) for SGXS conversion and signing.
> :warning: For SGXS conversion, change `--heap-size/--stack-size` value to `0x40000`, and `--threads 2` should be enough.

> :warning: For SGXS conversion and signing, the EDP instructions are shown for the "Debug" mode. For the production mode, remove the `--debug` / `-d` flags.
  
> :warning: For CPUs without the Flexible Launch Control feature (i.e. SGX v1), the enclave code needs to be signed with the RSA key previously approved by Intel in order to launch in the production mode.

#### Configuration

- Place the generated `*.sgxs` and `*.sig` files into `tmkms/enclave` directory.
- Place the `tmkms-light-sgx-runner` binary to `tmkms` directory.

*tmkms init*

Provide your chain `bech32_prefix`
```bash
$ tmkms-light-sgx-runner init -b bech32_prefix -p "bech32"
```

> :warning: For those who are running on Azure or other cloud environments, one may want to run `init` command with a cloud backup key.
> In cloud environments such as Azure, `CPU-affinity` may not be guaranteed, so SGX “sealing” (a way to encrypt the validator key that only a particular CPU can decrypt the validator key) may not be fully relied on. Please follow `With cloud backup key` if you intend to deploy in those settings.

<details>
  <summary>With cloud backup key</summary>

One may provide flag `-e backup_key_path` which is to encrypt and decrypt `consensus-key.backup` in directory specified in `-k backup_data_path`

One may also want to use [Azure Key Vault secret](https://docs.microsoft.com/en-us/azure/key-vault/secrets/quick-create-python) or its equivalents to store your backup key.

> :warning: After storing your backup key in Azure Key Vault or its equivalents, make sure to delete the backup key from any local environments to prevent its potential leakage.

> :warning: The instructions do not cover the adequate secure policy setup in Azure Key Vault or its equivalents, so make sure the policy restricts the backup key access only to particular instances during a recovery process.

> :warning: The current cloud key backup implementation is very rudimentary and potentially dangerous -- *use at your own risk* -- we plan to enhance this implementation once solutions for "cloud sealing" become generally available.
### Init
```bash
$ tmkms-light-sgx-runner init -b bech32_prefix -p "bech32" -e backup_key_path -k backup_data_path
```
Or follow the example python script to run [init](script/tmkms-sgx/init.py)

### Recover
```bash
$ tmkms-light-sgx-runner recover -b bech32_prefix -p "bech32" -e backup_key_path -k backup_data_path -r
```
Or follow the example python script to run [recover](script/tmkms-sgx/recover.py)
</details>

Lastly, edit the generated `tmkms.toml` to fit the target chain config, i.e chain_id and enclave_path
#### Running

*tmkms start*

```bash
tmkms-light-sgx-runner start
```
### AWS Nitro Enclaves
This is contained in the "providers/nitro" directory.
There are two crates that need to be compiled separately:
* `tmkms-nitro-enclave`: this is the enclave application that needs to be compiled for the `x86_64-unknown-linux-musl` target if you are using the Alpine Linux (or equivalent) for the Docker file that gets converted to the enclave image file. The enclave application also needs to be linked against AWS Nitro Enclaves SDK etc., so make sure these libraries are present in your build environment -- see this [AWS Dockerfile](https://github.com/aws/aws-nitro-enclaves-acm/blob/main/env/enclave/Dockerfile).
* `tmkms-nitro-helper`: this is the application on the host that pushes the configuration to the enclave and provides extra vsock proxy connections to interface with the host system.
#### Installation
TODO: instructions from the early prototype are here: https://chain.crypto.com/docs/getting-started/advanced-tmkms-integration.html#setting-up-aws-nitro-enclaves-tendermint-kms-for-signing-blocks
#### Configuration
TODO
#### Running
TODO
