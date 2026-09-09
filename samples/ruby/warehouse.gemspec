# frozen_string_literal: true

require_relative "lib/warehouse/version"

Gem::Specification.new do |spec|
  spec.name = "warehouse"
  spec.version = Warehouse::VERSION
  spec.authors = ["Example"]
  spec.email = ["engineering@example.com"]

  spec.summary = "Stock, money and audit trails for a small warehouse."
  spec.description = "An inventory with money handled in pennies, an audit " \
                     "mixin, and errors that name the stock keeping unit."
  spec.homepage = "https://github.com/example/warehouse"
  spec.license = "MIT"
  spec.required_ruby_version = ">= 3.2.0"

  spec.metadata["source_code_uri"] = "https://github.com/example/warehouse"
  spec.metadata["changelog_uri"] = "https://github.com/example/warehouse/blob/main/CHANGELOG.md"
  spec.metadata["rubygems_mfa_required"] = "true"

  spec.files = Dir["lib/**/*.rb", "README.md", "LICENSE"]
  spec.require_paths = ["lib"]
end
