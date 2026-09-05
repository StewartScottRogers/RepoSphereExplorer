import java.util.Objects;

public class Hello {
    private final String name;

    public Hello(String name) {
        this.name = name;
    }

    @Override
    public String toString() {
        return "Hello(" + name + ")";
    }

    public static void main(String[] args) {
        Hello hello = new Hello("World");
        System.out.println("Hello, " + hello.name + "!");
        System.out.println(Objects.toString(hello));
    }
}
