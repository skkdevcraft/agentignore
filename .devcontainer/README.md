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