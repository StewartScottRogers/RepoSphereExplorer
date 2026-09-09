Pod::Spec.new do |spec|
  spec.name         = "DownloadQueue"
  spec.version      = "1.2.0"
  spec.summary      = "A bounded download queue with retries and progress."
  spec.description  = <<-DESC
                      A queue that runs a fixed number of downloads at once,
                      retries the ones that fail, and reports progress as it
                      goes. No dependencies beyond Foundation.
                      DESC
  spec.homepage     = "https://github.com/example/DownloadQueue"
  spec.license      = { :type => "MIT", :file => "LICENSE" }
  spec.author       = { "Example" => "engineering@example.com" }

  spec.ios.deployment_target  = "15.0"
  spec.osx.deployment_target  = "12.0"

  spec.source       = { :git => "https://github.com/example/DownloadQueue.git",
                        :tag => "#{spec.version}" }
  spec.source_files = "Classes/**/*.{h,m}"
  spec.public_header_files = "Classes/**/*.h"
  spec.frameworks   = "Foundation"
  spec.requires_arc = true

  spec.test_spec "Tests" do |test_spec|
    test_spec.source_files = "Tests/**/*.m"
  end
end
