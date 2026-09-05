FROM alpine:3.19

WORKDIR /app

COPY hello.sh /app/hello.sh

RUN chmod +x /app/hello.sh

ENV GREETING="Hello, world!"

CMD ["/app/hello.sh"]
