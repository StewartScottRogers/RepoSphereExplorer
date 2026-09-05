case class Person(name: String)

object Hello extends App {
  val p = Person("World")
  println(s"Hello, ${p.name}")
}
