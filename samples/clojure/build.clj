(ns build
  "Builds the jar. Kept separate from deps.edn so the build is code you can
   read rather than configuration you have to infer."
  (:require [clojure.tools.build.api :as b]))

(def lib 'com.example/ledger)
(def version "1.0.0")
(def class-dir "target/classes")
(def jar-file (format "target/%s-%s.jar" (name lib) version))

(defn clean [_]
  (b/delete {:path "target"}))

(defn jar [_]
  (b/write-pom {:class-dir class-dir
                :lib lib
                :version version
                :basis (b/create-basis {:project "deps.edn"})
                :src-dirs ["src"]})
  (b/copy-dir {:src-dirs ["src" "resources"] :target-dir class-dir})
  (b/jar {:class-dir class-dir :jar-file jar-file}))
