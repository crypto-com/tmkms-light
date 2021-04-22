import cmd
import os
import secrets
import subprocess
import base64

from azure.identity import ManagedIdentityCredential
from azure.keyvault.secrets import SecretClient

tmkms_path = os.environ.get("TMKMS_RUNNER", "tmkms-light-sgx-runner")
backup_key_path = os.environ.get("BACKUP_KEY", "cloudbackup.key")
backup_data_path = os.environ.get("BACKUP_DATA", "/tmp/")
key_vault_name = os.environ["KEY_VAULT_NAME"]
backup_prefix = os.environ.get("CLOUD_BACKUP_PREFIX", "cloud-backup")
pubkey_display = os.environ.get("PUBKEY_DISPLAY", "bech32")
bech32_prefix = os.environ.get("BECH32_PREFIX", "crocnclconspub")
kv_uri = f"https://{key_vault_name}.vault.azure.net"
credential = ManagedIdentityCredential()
client = SecretClient(vault_url=kv_uri, credential=credential)
secret_name = backup_prefix + "-key"
secret_value = secrets.token_bytes(16)
base64_secret_value = base64.b64encode(secret_value).decode()
client.set_secret(secret_name, base64_secret_value)
try:
    with open(backup_key_path, "wb") as f:
        f.write(secret_value)
    os.chmod(backup_key_path, 0o400)
    command = [tmkms_path, "init", "-b", bech32_prefix, "-p", pubkey_display, "-e", backup_key_path, "-k", backup_data_path]
    subprocess.run(command)
finally:
    os.remove(backup_key_path)