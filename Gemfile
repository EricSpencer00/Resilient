# Gemfile for the docs/ Jekyll site. Lives at the repo root
# (NOT docs/) because the GitHub Pages auto-generated Actions
# workflow runs `bundle exec jekyll build --source docs ...`
# from the repo root, and bundler resolves Gemfile relative to
# the cwd. With the Gemfile at docs/, that workflow fails with
# "Could not locate Gemfile or .bundle/ directory".
#
# Local preview:
#   bundle install
#   bundle exec jekyll serve --source docs
#   → http://localhost:4000/Resilient/
source "https://rubygems.org"

# Pin to the same Jekyll version GH Pages uses so local previews
# match the deployed site.
gem "github-pages", "~> 232", group: :jekyll_plugins

# Theme is loaded via remote_theme in _config.yml — listed here
# too so `bundle exec jekyll serve` can find it locally.
gem "just-the-docs"

# Windows + JRuby quirks — harmless on macOS/Linux.
platforms :mingw, :x64_mingw, :mswin, :jruby do
  gem "tzinfo", ">= 1", "< 3"
  gem "tzinfo-data"
end
