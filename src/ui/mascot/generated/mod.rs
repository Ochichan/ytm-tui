pub mod cat_laptop;
pub mod dj_gem;
pub mod momoring;

static ALL_ASSETS: [&super::asset::MascotAsset; 20] = [
    &dj_gem::DJ_GEM_IDLE,
    &dj_gem::DJ_GEM_IDLE_RETRO,
    &dj_gem::DJ_GEM_GROOVE,
    &dj_gem::DJ_GEM_GROOVE_RETRO,
    &dj_gem::DJ_GEM_THINKING,
    &dj_gem::DJ_GEM_THINKING_RETRO,
    &cat_laptop::CAT_LAPTOP_IDLE,
    &cat_laptop::CAT_LAPTOP_IDLE_RETRO,
    &cat_laptop::CAT_LAPTOP_GROOVE,
    &cat_laptop::CAT_LAPTOP_GROOVE_RETRO,
    &momoring::MOMORING_IDLE_65X40,
    &momoring::MOMORING_WORKING_65X40,
    &momoring::MOMORING_IDLE_49X30,
    &momoring::MOMORING_WORKING_49X30,
    &momoring::MOMORING_IDLE_41X25,
    &momoring::MOMORING_WORKING_41X25,
    &momoring::MOMORING_IDLE_33X20,
    &momoring::MOMORING_WORKING_33X20,
    &momoring::MOMORING_IDLE_24X15,
    &momoring::MOMORING_WORKING_24X15,
];

pub fn all_assets() -> &'static [&'static super::asset::MascotAsset] {
    &ALL_ASSETS
}
