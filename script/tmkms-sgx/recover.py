import os
import base64
import subprocess

from azure.identity import ManagedIdentityCredential
from azure.keyvault.secrets import SecretClient

tmkms_path = os.environ.get("TMKMS_RUNNER", "tmkms-light-sgx-runner")
backup_key_path = os.environ.get("BACKUP_KEY", "cloudbackup.key")
backup_data_path = os.environ.get("BACKUP_DATA", "/tmp/")
recover_consensus_key = os.environ.get("RECOVER_CONSENSUS_KEY", None) != None
key_vault_name = os.environ["KEY_VAULT_NAME"]
backup_prefix = os.environ.get("CLOUD_BACKUP_PREFIX", "cloud-backup")
pubkey_display = os.environ.get("PUBKEY_DISPLAY", "bech32")
bech32_prefix = os.environ.get("BECH32_PREFIX", "crocnclconspub")
kv_uri = f"https://{key_vault_name}.vault.azure.net"
credential = ManagedIdentityCredential()
client = SecretClient(vault_url=kv_uri, credential=credential)
secret_name = backup_prefix + "-key"
base64_secret_value = client.get_secret(secret_name).value
secret_value = base64.b64decode(base64_secret_value.encode())
try:
    with open(backup_key_path, "wb") as f:
        f.write(secret_value) # change
    os.chmod(backup_key_path, 0o400)
    command = [tmkms_path, "recover", "-p", pubkey_display, "-b", bech32_prefix, "-e", backup_key_path, "-k", backup_data_path]
    if recover_consensus_key:
        command.append("-r")
    subprocess.run(command)
finally:
    os.remove(backup_key_path)