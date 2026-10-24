# frozen_string_literal: true

Gem::Specification.new do |spec|
  spec.name = "widgetco"
  spec.version = "2.3.0"
  spec.summary = "Ruby client for the Widgetco API"
  spec.description = "A widget catalogue whose list responses nest their paging markers under `list`."
  spec.authors = ["opensdk"]
  spec.license = "MIT"
  spec.required_ruby_version = ">= 2.6.0"
  spec.files = Dir["lib/**/*.rb", "*.gemspec"]
  spec.require_paths = ["lib"]
end
