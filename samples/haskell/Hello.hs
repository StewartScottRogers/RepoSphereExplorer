{-# LANGUAGE OverloadedStrings #-}
module Main (main) where

import qualified Data.Text as T
import qualified Data.Text.IO as TIO

greeting :: T.Text -> T.Text
greeting name = T.concat ["Hello, ", name, "!"]

main :: IO ()
main = TIO.putStrLn (greeting "World")
