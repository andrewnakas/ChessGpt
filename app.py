from flask import Flask, request, jsonify

app = Flask(__name__)

@app.route('/fen', methods=['POST'])
def receive_fen():
    # Extract FEN string from the received data
    fen_string = request.json.get('fen', '')
    if not fen_string:
        return jsonify({'error': 'No FEN string provided'}), 400

    # Here, add your logic to handle the FEN string
    # For example, interacting with your GitHub project
    # ...

    # Return a response
    return jsonify({'message': 'FEN string received successfully', 'fen': fen_string})

if __name__ == '__main__':
    app.run(debug=True, port=5000)
