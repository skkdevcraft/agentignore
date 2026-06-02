```bash
# install nvm
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.4/install.sh | bash

# install node
nvm install --lts

# install pi
npm install -g --ignore-scripts @earendil-works/pi-coding-agent

# install skills
npx skills@latest add mattpocock/skills

# open session in a container
docker exec -it -u vscode <container name> bash
```

```bash
sudo apt update
sudo apt install fuse3
```

```bash
sudo apt-get install gcc-aarch64-linux-gnu
sudo apt-get install gcc-x86-64-linux-gnu
sudp apt-get build-essentials

rustup target add aarch64-unknown-linux-gnu
rustup target add x86_64-unknown-linux-gnu

cargo build --release --target aarch64-unknown-linux-gnu
cargo build --release --target x86_64-unknown-linux-gnu

cargo npm generate
cargo npm publish

```