FROM python:3.13-alpine

ARG MODEL_BEHAVIOR=good
ENV MODEL_BEHAVIOR=${MODEL_BEHAVIOR}

WORKDIR /app
COPY server.py /app/server.py

EXPOSE 8080
CMD ["python", "/app/server.py"]
