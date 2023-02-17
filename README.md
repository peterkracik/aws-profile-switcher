# Simple AWS Profile switcher

Simplifies switching between multiple profiles, instead of writing `export AWS_PROFILE=profile1` just select it

![Screenshot 2023-02-17 at 13 14 36](https://user-images.githubusercontent.com/1544026/219651127-754a8f60-4f16-4fce-8a04-62f0c67592a6.png)

## Note
However, it is not allowed for the process to change environment variables, and thus it needs to be handle via bash eval fn


**Zsh**
```bash
# ~/.zshrc
awsprofile() {
  eval `/PATH/TO/aws-profile-switcher`
}
```

## Requirement
`aws` is installed and accessible


### using
https://docs.rs/dialoguer/latest/dialoguer/
