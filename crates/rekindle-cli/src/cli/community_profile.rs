use clap::{Args, ValueEnum};
use rekindle_types::dto::community_profile::{CommunityProfilePatch, ProfileField};

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ProfileKey {
    Bio,
    Pronouns,
    ThemeColor,
    Badges,
    AvatarRef,
    BannerRef,
    CustomStatus,
    DisplayName,
}

/// Edits to our profile in one community.
#[derive(Args)]
pub struct ProfileArgs {
    /// Community name or anchor key.
    #[arg(long, short = 'c')]
    pub community: String,
    #[arg(long)]
    bio: Option<String>,
    #[arg(long)]
    pronouns: Option<String>,
    /// ARGB colour as an unsigned 32-bit integer.
    #[arg(long)]
    theme_color: Option<u32>,
    /// Replace the badge list. Repeat to set more than one badge.
    #[arg(long)]
    badge: Vec<String>,
    #[arg(long)]
    avatar_ref: Option<String>,
    #[arg(long)]
    banner_ref: Option<String>,
    #[arg(long)]
    custom_status: Option<String>,
    #[arg(long)]
    display_name: Option<String>,
    /// Clear this field. Repeat to clear more than one field.
    #[arg(long, value_enum)]
    clear: Vec<ProfileKey>,
}

impl ProfileArgs {
    pub fn patch(&self) -> anyhow::Result<CommunityProfilePatch> {
        Ok(CommunityProfilePatch {
            bio: self.field(ProfileKey::Bio, self.bio.clone())?,
            pronouns: self.field(ProfileKey::Pronouns, self.pronouns.clone())?,
            theme_color: self.field(ProfileKey::ThemeColor, self.theme_color)?,
            badges: self.field(
                ProfileKey::Badges,
                (!self.badge.is_empty()).then(|| self.badge.clone()),
            )?,
            avatar_ref: self.field(ProfileKey::AvatarRef, self.avatar_ref.clone())?,
            banner_ref: self.field(ProfileKey::BannerRef, self.banner_ref.clone())?,
            custom_status: self.field(ProfileKey::CustomStatus, self.custom_status.clone())?,
            display_name: self.field(ProfileKey::DisplayName, self.display_name.clone())?,
        })
    }

    fn field<T>(&self, key: ProfileKey, value: Option<T>) -> anyhow::Result<ProfileField<T>> {
        let clears = self
            .clear
            .iter()
            .filter(|candidate| **candidate == key)
            .count();
        match (value, clears) {
            (None, 0) => Ok(ProfileField::Unchanged),
            (None, 1) => Ok(ProfileField::Clear),
            (Some(value), 0) => Ok(ProfileField::Set(value)),
            _ => anyhow::bail!("give one edit for each profile field"),
        }
    }
}
